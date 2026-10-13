//! Forwarded access tokens verified with the issuer's current keys.
//!
//! A hosted server starts from the keys its registration response names in
//! `veoveo_access_tokens`. When a token names a key the server does not hold, the
//! server reads the issuer's RFC 8414 metadata at
//! `/.well-known/oauth-authorization-server`, fetches the JWK Set its `jwks_uri` names
//! and verifies the token again, as RFC 9068 §4 describes. It fetches at most once per
//! [`KEY_REFRESH_INTERVAL`], bounds each request by [`KEY_FETCH_TIMEOUT`] and
//! [`KEY_DOCUMENT_LIMIT`], follows no redirect and keeps the keys in memory. A fetched
//! set replaces the cached keys only when it holds the key the token names, so an
//! issuer whose set lacks the forwarded-token keys cannot remove them.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use parking_lot::RwLock;
use reqwest::{StatusCode, header::ACCEPT};
use serde::{Deserialize, de::DeserializeOwned};
use tokio::{sync::Mutex, time::Instant};
use veoveo_mcp_ext_registration::{
    AccessTokenTrust, AuthorizationServerMetadata, DeclarationError, Ed25519Jwk, Ed25519JwkSet,
};

use super::{
    GatewayInternalIdentity, GatewayInternalTokenVerifier, GatewayInternalTrustBundle,
    InternalTokenError,
};
use crate::{ServerSlug, TokenIssuer};

/// The shortest time between two fetches of the issuer's keys.
pub const KEY_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
/// The longest one request for the issuer's metadata or keys may take.
pub const KEY_FETCH_TIMEOUT: Duration = Duration::from_secs(5);
/// The largest metadata or JWK Set document a server reads.
pub const KEY_DOCUMENT_LIMIT: usize = 64 * 1024;

/// Verifies the Gateway's forwarded access tokens, refreshing the issuer's keys when a
/// token names one the server does not hold.
#[derive(Debug)]
pub struct ForwardedTokenVerifier {
    current: RwLock<Arc<GatewayInternalTokenVerifier>>,
    refresh: Option<KeyRefresh>,
}

#[derive(Debug)]
struct KeyRefresh {
    source: IssuerKeySource,
    interval: Duration,
    /// When the last fetch started. Holding the lock makes concurrent requests for an
    /// unknown key wait for one fetch.
    last_attempt: Mutex<Option<Instant>>,
}

impl ForwardedTokenVerifier {
    /// Verify tokens addressed to `server` with the issuer, keys and audience a
    /// registration response names in `veoveo_access_tokens`, and fetch the issuer's
    /// keys through `http`. The caller installs a rustls crypto provider before it
    /// builds `http`.
    pub fn new(
        trust: &AccessTokenTrust,
        server: ServerSlug,
        http: reqwest::Client,
    ) -> Result<Self, InternalTokenError> {
        Self::with_refresh_interval(trust, server, http, KEY_REFRESH_INTERVAL)
    }

    pub(crate) fn with_refresh_interval(
        trust: &AccessTokenTrust,
        server: ServerSlug,
        http: reqwest::Client,
        interval: Duration,
    ) -> Result<Self, InternalTokenError> {
        let verifier = GatewayInternalTokenVerifier::from_access_tokens(trust, server)?;
        Ok(Self {
            current: RwLock::new(Arc::new(verifier)),
            refresh: Some(KeyRefresh {
                source: IssuerKeySource::new(http, &trust.issuer)?,
                interval,
                last_attempt: Mutex::new(None),
            }),
        })
    }

    /// A verifier whose keys never change, for the in-process test gateway.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn fixed(verifier: GatewayInternalTokenVerifier) -> Self {
        Self {
            current: RwLock::new(Arc::new(verifier)),
            refresh: None,
        }
    }

    /// Verify a forwarded access token. A token that names an unknown key is verified
    /// again after a refresh of the issuer's keys, when one is due.
    pub async fn verify(
        &self,
        bearer_token: &str,
    ) -> Result<GatewayInternalIdentity, InternalTokenError> {
        let key_id = match self.current().verify(bearer_token) {
            Err(InternalTokenError::UnknownKeyId(key_id)) => key_id,
            verified => return verified,
        };
        let Some(refresh) = &self.refresh else {
            return Err(InternalTokenError::UnknownKeyId(key_id));
        };
        let mut last_attempt = refresh.last_attempt.lock().await;
        // A fetch that finished while this request waited may have brought the key.
        match self.current().verify(bearer_token) {
            Err(InternalTokenError::UnknownKeyId(_)) => {}
            verified => return verified,
        }
        if last_attempt.is_some_and(|started| started.elapsed() < refresh.interval) {
            return Err(InternalTokenError::UnknownKeyId(key_id));
        }
        *last_attempt = Some(Instant::now());
        let keys = match refresh.source.fetch().await {
            Ok(keys) => keys,
            Err(error) => {
                tracing::warn!(
                    issuer = %refresh.source.issuer,
                    %error,
                    "forwarded-token key refresh failed"
                );
                return Err(InternalTokenError::UnknownKeyId(key_id));
            }
        };
        if !keys.key_ids().any(|published| published == key_id) {
            tracing::warn!(
                issuer = %refresh.source.issuer,
                "the issuer's JWK Set does not hold the key a forwarded token names"
            );
            return Err(InternalTokenError::UnknownKeyId(key_id));
        }
        let refreshed = Arc::new(self.current().with_trust_bundle(keys));
        *self.current.write() = refreshed.clone();
        drop(last_attempt);
        refreshed.verify(bearer_token)
    }

    /// The identifiers of the keys this verifier holds.
    #[cfg(test)]
    pub(crate) fn key_ids(&self) -> Vec<String> {
        self.current()
            .trust_bundle
            .key_ids()
            .map(str::to_owned)
            .collect()
    }

    fn current(&self) -> Arc<GatewayInternalTokenVerifier> {
        self.current.read().clone()
    }
}

/// The issuer's RFC 8414 metadata and the JWK Set it names.
#[derive(Debug)]
struct IssuerKeySource {
    http: reqwest::Client,
    issuer: url::Url,
    metadata: url::Url,
}

impl IssuerKeySource {
    fn new(http: reqwest::Client, issuer: &TokenIssuer) -> Result<Self, InternalTokenError> {
        let issuer =
            url::Url::parse(issuer.as_str()).map_err(|_| InternalTokenError::InvalidIssuer)?;
        Ok(Self {
            metadata: AuthorizationServerMetadata::url_for(&issuer),
            http,
            issuer,
        })
    }

    async fn fetch(&self) -> Result<GatewayInternalTrustBundle, KeyRefreshError> {
        let metadata: AuthorizationServerMetadata =
            self.document(&self.metadata, "application/json").await?;
        // RFC 8414 §3.3: the metadata names the issuer it was requested for.
        if metadata.issuer != self.issuer {
            return Err(KeyRefreshError::IssuerMismatch);
        }
        let published: PublishedKeySet = self
            .document(
                metadata.key_set_url()?,
                "application/jwk-set+json, application/json",
            )
            .await?;
        published.trust_bundle()
    }

    async fn document<T: DeserializeOwned>(
        &self,
        url: &url::Url,
        accept: &'static str,
    ) -> Result<T, KeyRefreshError> {
        let mut response = self
            .http
            .get(url.clone())
            .header(ACCEPT, accept)
            .timeout(KEY_FETCH_TIMEOUT)
            .send()
            .await?;
        if response.url() != url {
            return Err(KeyRefreshError::Redirected);
        }
        if response.status() != StatusCode::OK {
            return Err(KeyRefreshError::Status(response.status()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > KEY_DOCUMENT_LIMIT as u64)
        {
            return Err(KeyRefreshError::TooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len() > KEY_DOCUMENT_LIMIT {
                return Err(KeyRefreshError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(KeyRefreshError::Document)
    }
}

/// An RFC 7517 JWK Set as the issuer publishes it. As RFC 7517 §5 asks, entries the
/// server does not understand are ignored: only Ed25519 signature keys named by their
/// RFC 7638 thumbprint, in the shape of `veoveo_access_tokens`, become trusted.
#[derive(Deserialize)]
struct PublishedKeySet {
    keys: Vec<PublishedKey>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PublishedKey {
    Verification(Ed25519Jwk),
    Other(serde::de::IgnoredAny),
}

impl PublishedKeySet {
    fn trust_bundle(self) -> Result<GatewayInternalTrustBundle, KeyRefreshError> {
        let keys: BTreeMap<_, _> = self
            .keys
            .into_iter()
            .filter_map(|key| match key {
                PublishedKey::Verification(jwk) => Some((jwk.kid(), jwk)),
                PublishedKey::Other(_) => None,
            })
            .collect();
        let keys = Ed25519JwkSet::new(keys.into_values().collect())
            .map_err(|_| KeyRefreshError::NoVerificationKey)?;
        GatewayInternalTrustBundle::from_key_set(&keys).map_err(KeyRefreshError::Keys)
    }
}

/// Why a refresh of the issuer's keys failed. The server logs it and rejects the token.
#[derive(Debug, thiserror::Error)]
enum KeyRefreshError {
    #[error("the request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("the issuer answered HTTP {0}")]
    Status(StatusCode),
    #[error("the issuer redirected the request")]
    Redirected,
    #[error("the document exceeds {KEY_DOCUMENT_LIMIT} bytes")]
    TooLarge,
    #[error("the document does not decode: {0}")]
    Document(serde_json::Error),
    #[error("the metadata names another issuer")]
    IssuerMismatch,
    #[error(transparent)]
    Metadata(#[from] DeclarationError),
    #[error("the JWK Set holds no Ed25519 verification key")]
    NoVerificationKey,
    #[error(transparent)]
    Keys(InternalTokenError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal_auth::tests::signing_key;

    #[test]
    fn published_sets_keep_only_ed25519_verification_keys() {
        let key = signing_key().verification_key();
        let mut extended = serde_json::to_value(key).unwrap();
        extended["key_ops"] = serde_json::json!(["verify"]);
        let published: PublishedKeySet = serde_json::from_value(serde_json::json!({
            "keys": [
                {"kty": "RSA", "kid": "rsa-1", "n": "AQAB", "e": "AQAB", "alg": "RS256"},
                extended,
                key,
                key,
                {"kty": "OKP", "crv": "Ed25519", "x": key.key().x(), "kid": "not-a-thumbprint", "alg": "EdDSA", "use": "sig"}
            ],
            "unrelated": true
        }))
        .unwrap();
        let bundle = published.trust_bundle().unwrap();
        assert_eq!(bundle.key_ids().collect::<Vec<_>>(), [key.kid().as_str()]);
        let foreign: PublishedKeySet = serde_json::from_value(serde_json::json!({
            "keys": [{"kty": "RSA", "kid": "rsa-1", "n": "AQAB", "e": "AQAB"}]
        }))
        .unwrap();
        assert!(matches!(
            foreign.trust_bundle(),
            Err(KeyRefreshError::NoVerificationKey)
        ));
        assert!(serde_json::from_value::<PublishedKeySet>(serde_json::json!({})).is_err());
    }
}
