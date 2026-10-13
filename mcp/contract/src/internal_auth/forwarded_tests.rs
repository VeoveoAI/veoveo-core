//! Key refresh against an issuer on a loopback listener that serves RFC 8414 metadata
//! and a JWK Set.
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, header::LOCATION},
    routing::get,
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use veoveo_mcp_ext_registration::{AccessTokenTrust, Ed25519JwkSet, statement::ServerKeyPair};

use super::{
    ForwardedTokenVerifier, GatewayInternalSigningKey, GatewayInternalTokenIssuer,
    InternalTokenError, KEY_DOCUMENT_LIMIT, KEY_REFRESH_INTERVAL,
    tests::{forwarded, resource},
};
use crate::{ServerSlug, TokenIssuer};

/// An HTTP client for loopback requests, with the process's rustls provider installed.
pub(crate) fn http() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::new()
}

/// A fresh Ed25519 signing key.
pub(crate) fn generated_key() -> GatewayInternalSigningKey {
    GatewayInternalSigningKey::new(ServerKeyPair::generate().unwrap().pkcs8_der()).unwrap()
}

#[derive(Clone, Default)]
struct Published {
    metadata: Arc<Mutex<Value>>,
    jwks: Arc<Mutex<Value>>,
    jwks_requests: Arc<AtomicUsize>,
}

/// A Gateway issuer on a loopback port: its metadata names its JWK Set at
/// `/oauth/jwks.json`.
pub(crate) struct KeyServer {
    pub(crate) issuer: TokenIssuer,
    published: Published,
    task: tokio::task::JoinHandle<()>,
}

impl KeyServer {
    pub(crate) async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let published = Published::default();
        *published.metadata.lock() = json!({
            "issuer": issuer,
            "jwks_uri": format!("{issuer}/oauth/jwks.json"),
        });
        *published.jwks.lock() = json!({"keys": []});
        let router = Router::new()
            .route(
                "/.well-known/oauth-authorization-server",
                get(|State(published): State<Published>| async move {
                    Json(published.metadata.lock().clone())
                }),
            )
            .route(
                "/oauth/jwks.json",
                get(|State(published): State<Published>| async move {
                    published.jwks_requests.fetch_add(1, Ordering::SeqCst);
                    Json(published.jwks.lock().clone())
                }),
            )
            .route(
                "/oauth/elsewhere.json",
                get(|State(published): State<Published>| async move {
                    Json(published.jwks.lock().clone())
                }),
            )
            .route(
                "/moved",
                get(|| async { (StatusCode::FOUND, [(LOCATION, "/oauth/elsewhere.json")]) }),
            )
            .route(
                "/large",
                get(|| async {
                    format!(
                        "{{\"keys\":[],\"pad\":\"{}\"}}",
                        "x".repeat(KEY_DOCUMENT_LIMIT)
                    )
                }),
            )
            .with_state(published.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            issuer: TokenIssuer::parse(&issuer).unwrap(),
            published,
            task,
        }
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{path}", self.issuer.as_str())
    }

    /// Publish `keys` as the issuer's JWK Set.
    pub(crate) fn publish(&self, keys: &[&GatewayInternalSigningKey]) {
        *self.published.jwks.lock() = serde_json::to_value(key_set(keys)).unwrap();
    }

    pub(crate) fn set_metadata(&self, metadata: Value) {
        *self.published.metadata.lock() = metadata;
    }

    pub(crate) fn jwks_requests(&self) -> usize {
        self.published.jwks_requests.load(Ordering::SeqCst)
    }

    /// What a registration response names for test server `server`: this issuer,
    /// `keys` and the server's resource URL.
    pub(crate) fn trust(
        &self,
        keys: &[&GatewayInternalSigningKey],
        server: &str,
    ) -> AccessTokenTrust {
        AccessTokenTrust {
            issuer: self.issuer.clone(),
            jwks: key_set(keys),
            resource: resource(server),
        }
    }

    /// A forwarded token for test server `server`, signed by `key`.
    pub(crate) fn token(&self, key: &GatewayInternalSigningKey, server: &str) -> String {
        GatewayInternalTokenIssuer::new(self.issuer.clone(), key.clone())
            .unwrap()
            .issue(forwarded(server, &[]))
            .unwrap()
            .bearer_token
    }
}

impl Drop for KeyServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn key_set(keys: &[&GatewayInternalSigningKey]) -> Ed25519JwkSet {
    Ed25519JwkSet::new(keys.iter().map(|key| key.verification_key()).collect()).unwrap()
}

fn verifier(
    server: &KeyServer,
    trusted: &[&GatewayInternalSigningKey],
    interval: Duration,
) -> ForwardedTokenVerifier {
    ForwardedTokenVerifier::with_refresh_interval(
        &server.trust(trusted, "time"),
        ServerSlug::parse("time").unwrap(),
        http(),
        interval,
    )
    .unwrap()
}

fn unknown_key(result: Result<crate::GatewayInternalIdentity, InternalTokenError>) -> bool {
    matches!(result, Err(InternalTokenError::UnknownKeyId(_)))
}

#[tokio::test]
async fn rotated_keys_come_from_the_issuers_jwks_uri() {
    let server = KeyServer::start().await;
    let (registered, rotated) = (generated_key(), generated_key());
    server.publish(&[&registered, &rotated]);
    let verifier = verifier(&server, &[&registered], KEY_REFRESH_INTERVAL);
    verifier
        .verify(&server.token(&registered, "time"))
        .await
        .unwrap();
    assert_eq!(server.jwks_requests(), 0, "a known key needs no fetch");
    let identity = verifier
        .verify(&server.token(&rotated, "time"))
        .await
        .unwrap();
    assert_eq!(identity.issuer, server.issuer);
    assert_eq!(server.jwks_requests(), 1);
    // The issuer publishes both keys, so both stay trusted without another fetch.
    for key in [&registered, &rotated] {
        verifier.verify(&server.token(key, "time")).await.unwrap();
    }
    assert_eq!(server.jwks_requests(), 1);
}

#[tokio::test]
async fn concurrent_requests_share_one_fetch_and_unknown_keys_wait_for_the_interval() {
    let server = KeyServer::start().await;
    let (registered, rotated, unknown) = (generated_key(), generated_key(), generated_key());
    server.publish(&[&rotated]);
    let verifier = verifier(&server, &[&registered], KEY_REFRESH_INTERVAL);
    let token = server.token(&rotated, "time");
    let verified = futures::future::join_all((0..8).map(|_| verifier.verify(&token))).await;
    assert!(verified.iter().all(Result::is_ok));
    assert_eq!(server.jwks_requests(), 1);
    // Within the interval a key the issuer never published causes no fetch.
    for _ in 0..3 {
        assert!(unknown_key(
            verifier.verify(&server.token(&unknown, "time")).await
        ));
    }
    assert_eq!(server.jwks_requests(), 1);
}

#[tokio::test]
async fn fetches_repeat_only_after_the_interval() {
    let server = KeyServer::start().await;
    let (registered, unknown) = (generated_key(), generated_key());
    server.publish(&[&registered]);
    let interval = Duration::from_millis(300);
    let verifier = verifier(&server, &[&registered], interval);
    let token = server.token(&unknown, "time");
    assert!(unknown_key(verifier.verify(&token).await));
    assert!(unknown_key(verifier.verify(&token).await));
    assert_eq!(server.jwks_requests(), 1);
    tokio::time::sleep(interval).await;
    assert!(unknown_key(verifier.verify(&token).await));
    assert_eq!(server.jwks_requests(), 2);
}

#[tokio::test]
async fn a_fetched_set_replaces_the_keys_only_when_it_holds_the_named_key() {
    let server = KeyServer::start().await;
    let (registered, rotated, unknown) = (generated_key(), generated_key(), generated_key());
    server.publish(&[&rotated]);
    let verifier = verifier(&server, &[&registered], Duration::ZERO);
    // A set without the token's key leaves the registration's key trusted.
    assert!(unknown_key(
        verifier.verify(&server.token(&unknown, "time")).await
    ));
    assert_eq!(verifier.key_ids(), [registered.key_id()]);
    verifier
        .verify(&server.token(&registered, "time"))
        .await
        .unwrap();
    // A set that holds it replaces the keys, which retires the registration's key.
    verifier
        .verify(&server.token(&rotated, "time"))
        .await
        .unwrap();
    assert_eq!(verifier.key_ids(), [rotated.key_id()]);
    assert!(unknown_key(
        verifier.verify(&server.token(&registered, "time")).await
    ));
    // Other failures never fetch.
    let requests = server.jwks_requests();
    let other_server = server.token(&rotated, "map");
    assert!(verifier.verify(&other_server).await.is_err());
    assert!(verifier.verify("not-a-token").await.is_err());
    assert_eq!(server.jwks_requests(), requests);
}

#[tokio::test]
async fn keys_come_only_from_the_jwk_set_the_issuers_metadata_names() {
    let server = KeyServer::start().await;
    let (registered, rotated) = (generated_key(), generated_key());
    server.publish(&[&rotated]);
    let verifier = verifier(&server, &[&registered], Duration::ZERO);
    let token = server.token(&rotated, "time");
    let issuer = server.issuer.as_str().to_owned();
    for metadata in [
        json!({"issuer": "https://gateway.example", "jwks_uri": server.url("/oauth/jwks.json")}),
        json!({"issuer": issuer}),
        json!({"issuer": issuer, "jwks_uri": "http://gateway.example/oauth/jwks.json"}),
        json!({"issuer": issuer, "jwks_uri": server.url("/moved")}),
        json!({"issuer": issuer, "jwks_uri": server.url("/large")}),
        json!({"issuer": issuer, "jwks_uri": server.url("/missing")}),
    ] {
        server.set_metadata(metadata.clone());
        assert!(unknown_key(verifier.verify(&token).await), "{metadata}");
        assert_eq!(verifier.key_ids(), [registered.key_id()], "{metadata}");
    }
    assert_eq!(server.jwks_requests(), 0);
    server.set_metadata(json!({"issuer": issuer, "jwks_uri": server.url("/oauth/jwks.json")}));
    verifier.verify(&token).await.unwrap();
    assert_eq!(server.jwks_requests(), 1);
}

#[tokio::test]
async fn an_unreachable_issuer_leaves_the_registered_keys() {
    let server = KeyServer::start().await;
    let (registered, rotated) = (generated_key(), generated_key());
    let verifier = verifier(&server, &[&registered], Duration::ZERO);
    let token = server.token(&rotated, "time");
    drop(server);
    tokio::task::yield_now().await;
    let started = tokio::time::Instant::now();
    assert!(unknown_key(verifier.verify(&token).await));
    assert!(started.elapsed() < super::KEY_FETCH_TIMEOUT * 2);
    assert_eq!(verifier.key_ids(), [registered.key_id()]);
}
