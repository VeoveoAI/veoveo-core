//! The registration client against an in-process RFC 7591 and RFC 7592 endpoint that
//! applies the Gateway's admission rules: the initial access token, the software
//! statement, namespace pinning and registration access tokens.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{TimeDelta, Utc};
use secrecy::SecretString;
use serde_json::json;
use veoveo_mcp_ext_registration::{
    client::{ClientError, KeyDirectory, RegistrationClient},
    statement::{ServerKeyPair, verify_statement},
    *,
};
use veoveo_types::{OAuthClientId, ResourceScheme, ServerSlug, TokenIssuer};

const SECRET: &str = "installation-registration-secret";

#[derive(Default)]
struct Gateway {
    base: String,
    pinned: HashMap<ServerSlug, ServerPublicKey>,
    clients: HashMap<String, (String, ClientMetadata)>,
    issued: usize,
    renewals: usize,
}

type Shared = Arc<Mutex<Gateway>>;

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

fn error(code: RegistrationErrorCode, description: String) -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(RegistrationErrorResponse {
            error: code,
            error_description: Some(description),
        }),
    )
        .into_response()
}

/// The Gateway's admission steps; `Some` is the rejection to send.
fn admit(
    gateway: &mut Gateway,
    metadata: &ClientMetadata,
    statement: &SoftwareStatement,
    audience: &str,
) -> Option<Response> {
    if let Err(e) = metadata.check() {
        return Some(error(
            RegistrationErrorCode::InvalidClientMetadata,
            e.to_string(),
        ));
    }
    if let Err(e) = verify_statement(
        statement,
        metadata,
        gateway.pinned.get(metadata.slug()),
        &url::Url::parse(audience).unwrap(),
        Utc::now(),
    ) {
        return Some(error(e.error_code(), e.to_string()));
    }
    gateway
        .pinned
        .entry(metadata.slug().clone())
        .or_insert(*metadata.key());
    None
}

fn information(
    gateway: &mut Gateway,
    client_id: &str,
    metadata: ClientMetadata,
    statement: SoftwareStatement,
    database: bool,
) -> ClientInformation {
    gateway.issued += 1;
    let token = format!("rat-{}", gateway.issued);
    gateway
        .clients
        .insert(client_id.to_owned(), (token.clone(), metadata.clone()));
    let grant = || {
        serde_json::from_value(json!({
            "endpoint": "ws://surrealdb:8000/",
            "namespace": "veoveo",
            "database": "platform",
            "username": format!("{}_runtime", metadata.slug()),
            "password": "issued-password"
        }))
        .unwrap()
    };
    ClientInformation {
        client_id: OAuthClientId::parse(client_id).unwrap(),
        client_id_issued_at: Utc::now(),
        registration_access_token: RegistrationAccessToken::new(token),
        registration_client_uri: url::Url::parse(&format!("{}/register/{client_id}", gateway.base))
            .unwrap(),
        database: (database && metadata.module.is_some()).then(grant),
        metadata,
        software_statement: statement,
        lease: RegistrationLease {
            expires_at: Utc::now() + TimeDelta::seconds(3),
            renew_every_seconds: 1,
        },
        access_tokens: AccessTokenTrust {
            issuer: TokenIssuer::parse("veoveo-internal").unwrap(),
            jwks: Ed25519JwkSet::new(vec![Ed25519Jwk {
                kid: JwkKeyId::new("gateway-1").unwrap(),
                key: ServerPublicKey::from_bytes([9; 32]),
            }])
            .unwrap(),
        },
    }
}

async fn register(State(gateway): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    if bearer(&headers) != Some(SECRET) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(request) = serde_json::from_slice::<ClientRegistrationRequest>(&body) else {
        return error(
            RegistrationErrorCode::InvalidClientMetadata,
            "malformed".into(),
        );
    };
    let mut gateway = gateway.lock().unwrap();
    let audience = format!("{}/register", gateway.base);
    if let Some(response) = admit(
        &mut gateway,
        &request.metadata,
        &request.software_statement,
        &audience,
    ) {
        return response;
    }
    let client_id = format!("{}-{}", request.metadata.slug(), gateway.issued);
    let created = information(
        &mut gateway,
        &client_id,
        request.metadata,
        request.software_statement,
        true,
    );
    (StatusCode::CREATED, axum::Json(created)).into_response()
}

fn authorized(gateway: &Gateway, id: &str, headers: &HeaderMap) -> Option<ClientMetadata> {
    let (token, metadata) = gateway.clients.get(id)?;
    (bearer(headers) == Some(token)).then(|| metadata.clone())
}

async fn read(
    State(gateway): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let mut gateway = gateway.lock().unwrap();
    let Some(metadata) = authorized(&gateway, &id, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let statement = SoftwareStatement::new("e30.e30.c2ln").unwrap();
    let read = information(&mut gateway, &id, metadata, statement, false);
    axum::Json(read).into_response()
}

async fn update(
    State(gateway): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let mut gateway = gateway.lock().unwrap();
    if authorized(&gateway, &id, &headers).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(request) = serde_json::from_slice::<ClientUpdateRequest>(&body) else {
        return error(
            RegistrationErrorCode::InvalidClientMetadata,
            "malformed".into(),
        );
    };
    if request.client_id.as_str() != id {
        return error(
            RegistrationErrorCode::InvalidClientMetadata,
            "client_id".into(),
        );
    }
    let audience = format!("{}/register/{id}", gateway.base);
    if let Some(response) = admit(
        &mut gateway,
        &request.metadata,
        &request.software_statement,
        &audience,
    ) {
        return response;
    }
    gateway.renewals += 1;
    let renewed = information(
        &mut gateway,
        &id,
        request.metadata,
        request.software_statement,
        false,
    );
    axum::Json(renewed).into_response()
}

async fn delete(
    State(gateway): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> StatusCode {
    let mut gateway = gateway.lock().unwrap();
    if authorized(&gateway, &id, &headers).is_none() {
        return StatusCode::UNAUTHORIZED;
    }
    gateway.clients.remove(&id);
    StatusCode::NO_CONTENT
}

async fn gateway() -> (Shared, url::Url) {
    let shared = Shared::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    shared.lock().unwrap().base = base.clone();
    let app = Router::new()
        .route("/register", post(register))
        .route("/register/{id}", get(read).put(update).delete(delete))
        .with_state(shared.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (
        shared,
        url::Url::parse(&format!("{base}/register")).unwrap(),
    )
}

fn http() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::new()
}

fn module() -> ModuleDeclaration {
    ModuleDeclaration::new(
        serde_json::from_value(json!({
            "name": "clock",
            "ownership": [{"kind": "tablePrefix", "name": "clock_"}],
            "migrations": [{"version": 0, "name": "current", "sql": "DEFINE TABLE clock_event;"}]
        }))
        .unwrap(),
    )
}

fn client(endpoint: &url::Url, key: ServerKeyPair, secret: &str) -> RegistrationClient {
    let metadata = ClientMetadata::new(
        *key.public_key(),
        RegistrationDeclaration {
            slug: ServerSlug::parse("clock").unwrap(),
            uri_scheme: ResourceScheme::parse("clock").unwrap(),
            version: ServerVersion::new("0.1.0").unwrap(),
            endpoint: ServerEndpoint::new(url::Url::parse("http://clock:8800/mcp").unwrap())
                .unwrap(),
        },
        AuthorizationDeclaration {
            permissions: Vec::new(),
            requirements: Requirements::default(),
        },
        Some(module()),
    )
    .unwrap();
    RegistrationClient::new(
        http(),
        endpoint.clone(),
        SecretString::from(secret.to_owned()),
        key,
        metadata,
    )
    .unwrap()
}

fn scratch() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("veoveo-registration-{}", uuid::Uuid::new_v4()))
}

#[test]
fn server_keys_persist_privately_in_their_directory() {
    let directory = scratch();
    let keys = KeyDirectory::new(directory.join("keys"));
    let first = keys.load_or_generate().unwrap();
    let second = keys.load_or_generate().unwrap();
    assert_eq!(first.public_key(), second.public_key());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(keys.key_path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    std::fs::write(keys.key_path(), b"not a key").unwrap();
    assert!(keys.load_or_generate().is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn registers_reads_renews_and_deregisters() {
    let (shared, endpoint) = gateway().await;
    let client = client(&endpoint, ServerKeyPair::generate().unwrap(), SECRET);
    let mut registration = client.register().await.unwrap();
    let database = registration.database().unwrap();
    assert_eq!(database.username.as_str(), "clock_runtime");
    assert_eq!(
        registration.access_tokens().issuer.as_str(),
        "veoveo-internal"
    );
    let first_token = registration
        .information()
        .registration_access_token
        .expose_secret()
        .to_owned();

    client.renew(&mut registration).await.unwrap();
    assert_eq!(shared.lock().unwrap().renewals, 1);
    assert_ne!(
        registration
            .information()
            .registration_access_token
            .expose_secret(),
        first_token,
        "the client adopts a rotated registration access token"
    );
    assert!(
        registration.database().is_some(),
        "the database user survives renewals"
    );

    let read = client.read(&registration).await.unwrap();
    assert_eq!(read.client_id(), registration.client_id());

    let stale = read.clone();
    client.deregister(read).await.unwrap();
    assert!(shared.lock().unwrap().clients.is_empty());
    assert!(matches!(
        client.read(&stale).await,
        Err(ClientError::Rejected {
            status: reqwest::StatusCode::UNAUTHORIZED,
            ..
        })
    ));
}

#[tokio::test]
async fn an_expired_registration_registers_again() {
    let (shared, endpoint) = gateway().await;
    let client = client(&endpoint, ServerKeyPair::generate().unwrap(), SECRET);
    let mut registration = client.register().await.unwrap();
    let expired = registration.client_id().clone();
    shared.lock().unwrap().clients.clear();
    client.renew(&mut registration).await.unwrap();
    assert_ne!(registration.client_id(), &expired);
    assert_eq!(shared.lock().unwrap().clients.len(), 1);
}

#[tokio::test]
async fn the_namespace_stays_pinned_to_its_first_key() {
    let (_, endpoint) = gateway().await;
    client(&endpoint, ServerKeyPair::generate().unwrap(), SECRET)
        .register()
        .await
        .unwrap();
    let impostor = client(&endpoint, ServerKeyPair::generate().unwrap(), SECRET);
    match impostor.register().await {
        Err(ClientError::Rejected {
            status: reqwest::StatusCode::BAD_REQUEST,
            error: Some(error),
        }) => assert_eq!(
            error.error,
            RegistrationErrorCode::UnapprovedSoftwareStatement
        ),
        other => panic!("expected a pinned-key rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn registration_requires_the_initial_access_token() {
    let (_, endpoint) = gateway().await;
    let client = client(&endpoint, ServerKeyPair::generate().unwrap(), "wrong");
    let rejected = client.register().await.unwrap_err();
    assert!(matches!(
        rejected,
        ClientError::Rejected {
            status: reqwest::StatusCode::UNAUTHORIZED,
            ..
        }
    ));
    assert!(!rejected.is_transient());
}

#[tokio::test]
async fn maintain_renews_until_shutdown_then_deregisters() {
    let (shared, endpoint) = gateway().await;
    let client = client(&endpoint, ServerKeyPair::generate().unwrap(), SECRET);
    let registration = client.register().await.unwrap();
    let watched = shared.clone();
    let shutdown = async move {
        while watched.lock().unwrap().renewals < 2 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    tokio::time::timeout(
        Duration::from_secs(10),
        client.maintain(registration, shutdown),
    )
    .await
    .expect("maintain finishes after shutdown")
    .unwrap();
    let gateway = shared.lock().unwrap();
    assert!(gateway.renewals >= 2);
    assert!(
        gateway.clients.is_empty(),
        "shutdown removes the registration"
    );
}
