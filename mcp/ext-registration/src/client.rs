//! The registration client a server runs. It keeps the server's key in a directory,
//! registers with the installation's initial access token (RFC 7591), and removes the
//! registration on shutdown (RFC 7592).
//!
//! The caller supplies the `reqwest::Client`. A client that reaches the Gateway over
//! HTTPS needs a process-wide rustls crypto provider installed first.
use std::{
    future::Future,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::Utc;
use reqwest::{
    Method, StatusCode,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE},
};
use secrecy::{ExposeSecret, SecretString};
use veoveo_types::OAuthClientId;

use crate::{
    AccessTokenTrust, AuthorizationServerMetadata, ClientInformation, ClientMetadata,
    ClientRegistrationRequest, ClientUpdateRequest, DatabaseGrant, DeclarationError,
    RegistrationErrorResponse,
    statement::{ServerKeyPair, StatementError},
};

/// The file inside a [`KeyDirectory`] that holds the server's PKCS#8 private key.
pub const KEY_FILE: &str = "server-key.p8";
/// The longest the client waits for one Gateway response.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("server key file {path}: {source}")]
    KeyFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Key(#[from] StatementError),
    #[error(transparent)]
    Metadata(#[from] DeclarationError),
    #[error("registration request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("the Gateway answered HTTP {status}{}", describe(.error))]
    Rejected {
        status: StatusCode,
        error: Option<RegistrationErrorResponse>,
    },
    #[error("the Gateway returned an invalid registration response: {0}")]
    InvalidResponse(serde_json::Error),
    #[error("the registration response {0}")]
    ResponseMismatch(&'static str),
}

fn describe(error: &Option<RegistrationErrorResponse>) -> String {
    match error {
        Some(RegistrationErrorResponse {
            error,
            error_description: Some(description),
        }) => format!(": {error}: {description}"),
        Some(RegistrationErrorResponse { error, .. }) => format!(": {error}"),
        None => String::new(),
    }
}

impl ClientError {
    /// Whether a later attempt can succeed without a change to the server.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Transport(_) => true,
            Self::Rejected { status, .. } => status.is_server_error(),
            _ => false,
        }
    }
}

/// A directory that holds the server's private key across restarts.
#[derive(Debug, Clone)]
pub struct KeyDirectory(PathBuf);

impl KeyDirectory {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    pub fn key_path(&self) -> PathBuf {
        self.0.join(KEY_FILE)
    }

    /// Load the server's key, generating and storing one on first start. The file is
    /// readable only by its owner and appears atomically.
    pub fn load_or_generate(&self) -> Result<ServerKeyPair, ClientError> {
        let path = self.key_path();
        match std::fs::read(&path) {
            Ok(der) => Ok(ServerKeyPair::from_pkcs8_der(&der)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let key = ServerKeyPair::generate()?;
                self.store(&key).map_err(|source| ClientError::KeyFile {
                    path: path.clone(),
                    source,
                })?;
                Ok(key)
            }
            Err(source) => Err(ClientError::KeyFile { path, source }),
        }
    }

    fn store(&self, key: &ServerKeyPair) -> std::io::Result<()> {
        create_private_dir(&self.0)?;
        let staged = self.0.join(format!(".{KEY_FILE}.{}", uuid::Uuid::new_v4()));
        let mut file = private_file(&staged)?;
        file.write_all(key.pkcs8_der())?;
        file.sync_all()?;
        std::fs::rename(&staged, self.key_path())
    }
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}
#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

#[cfg(unix)]
fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}
#[cfg(not(unix))]
fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// An admitted registration and what the Gateway issued with it.
#[derive(Debug, Clone)]
pub struct Registration {
    information: ClientInformation,
    database: Option<DatabaseGrant>,
}

impl Registration {
    pub fn client_id(&self) -> &OAuthClientId {
        &self.information.client_id
    }
    /// The issuer and keys that verify the access tokens the Gateway forwards.
    pub fn access_tokens(&self) -> &AccessTokenTrust {
        &self.information.access_tokens
    }
    /// The server's database user, kept from the response that last carried it.
    pub fn database(&self) -> Option<&DatabaseGrant> {
        self.database.as_ref()
    }
    pub fn information(&self) -> &ClientInformation {
        &self.information
    }
}

/// Registers one server with one Gateway.
pub struct RegistrationClient {
    http: reqwest::Client,
    endpoint: url::Url,
    initial_access_token: SecretString,
    key: ServerKeyPair,
    metadata: ClientMetadata,
    request_timeout: Duration,
}

impl std::fmt::Debug for RegistrationClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistrationClient")
            .field("endpoint", &self.endpoint.as_str())
            .field("server", self.metadata.slug())
            .finish_non_exhaustive()
    }
}

impl RegistrationClient {
    /// `endpoint` is the Gateway's RFC 7591 registration endpoint, and
    /// `initial_access_token` the installation's registration secret, or the
    /// placeholder an environment substitutes for it.
    pub fn new(
        http: reqwest::Client,
        endpoint: url::Url,
        initial_access_token: SecretString,
        key: ServerKeyPair,
        metadata: ClientMetadata,
    ) -> Result<Self, ClientError> {
        crate::registration::http_url(&endpoint, "registration_endpoint")?;
        metadata.check()?;
        if metadata.key() != key.public_key() {
            return Err(ClientError::Key(StatementError::Rejected(
                "metadata names another key than the signer",
            )));
        }
        Ok(Self {
            http,
            endpoint,
            initial_access_token,
            key,
            metadata,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        })
    }

    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Read the registration endpoint from the Gateway's RFC 8414 metadata.
    pub async fn discover(
        http: &reqwest::Client,
        issuer: &url::Url,
    ) -> Result<url::Url, ClientError> {
        let response = http
            .get(AuthorizationServerMetadata::url_for(issuer))
            .header(ACCEPT, "application/json")
            .timeout(DEFAULT_REQUEST_TIMEOUT)
            .send()
            .await?;
        let status = response.status();
        let body = response.bytes().await?;
        if status != StatusCode::OK {
            return Err(ClientError::Rejected {
                status,
                error: None,
            });
        }
        let metadata: AuthorizationServerMetadata =
            serde_json::from_slice(&body).map_err(ClientError::InvalidResponse)?;
        if &metadata.issuer != issuer {
            return Err(ClientError::ResponseMismatch(
                "names another issuer than the one discovered",
            ));
        }
        metadata
            .registration_endpoint
            .ok_or(ClientError::ResponseMismatch(
                "advertises no registration endpoint",
            ))
    }

    pub fn metadata(&self) -> &ClientMetadata {
        &self.metadata
    }

    /// RFC 7591 registration.
    pub async fn register(&self) -> Result<Registration, ClientError> {
        let request = ClientRegistrationRequest {
            metadata: self.metadata.clone(),
            software_statement: self.key.sign_statement(
                &self.metadata,
                &self.endpoint,
                Utc::now(),
            )?,
        };
        let information = self
            .exchange(
                Method::POST,
                &self.endpoint,
                self.initial_access_token.expose_secret(),
                Some(serde_json::to_vec(&request).expect("request serializes")),
                StatusCode::CREATED,
            )
            .await?;
        self.admit(information, None)
    }

    /// RFC 7592 read.
    pub async fn read(&self, registration: &Registration) -> Result<Registration, ClientError> {
        let information = self
            .exchange(
                Method::GET,
                &registration.information.registration_client_uri,
                registration
                    .information
                    .registration_access_token
                    .expose_secret(),
                None,
                StatusCode::OK,
            )
            .await?;
        self.admit(information, registration.database.clone())
    }

    /// RFC 7592 update: replace the registered metadata with this client's.
    pub async fn update(&self, registration: &mut Registration) -> Result<(), ClientError> {
        let uri = registration.information.registration_client_uri.clone();
        let request = ClientUpdateRequest {
            client_id: registration.information.client_id.clone(),
            metadata: self.metadata.clone(),
            software_statement: self.key.sign_statement(&self.metadata, &uri, Utc::now())?,
        };
        let information = self
            .exchange(
                Method::PUT,
                &uri,
                registration
                    .information
                    .registration_access_token
                    .expose_secret(),
                Some(serde_json::to_vec(&request).expect("request serializes")),
                StatusCode::OK,
            )
            .await?;
        *registration = self.admit(information, registration.database.clone())?;
        Ok(())
    }

    /// RFC 7592 delete.
    pub async fn deregister(&self, registration: Registration) -> Result<(), ClientError> {
        let response = self
            .send(
                Method::DELETE,
                &registration.information.registration_client_uri,
                registration
                    .information
                    .registration_access_token
                    .expose_secret(),
                None,
            )
            .await?;
        match response.status() {
            StatusCode::NO_CONTENT | StatusCode::UNAUTHORIZED => Ok(()),
            status => Err(rejected(status, &response.bytes().await?)),
        }
    }

    /// Keep the registration until `shutdown` completes, then remove it. Between the
    /// two the Gateway keeps the registration while the server's login holds a live
    /// kernel instance.
    pub async fn maintain(
        &self,
        registration: Registration,
        shutdown: impl Future<Output = ()>,
    ) -> Result<(), ClientError> {
        shutdown.await;
        self.deregister(registration).await
    }

    async fn exchange(
        &self,
        method: Method,
        url: &url::Url,
        bearer: &str,
        body: Option<Vec<u8>>,
        expected: StatusCode,
    ) -> Result<ClientInformation, ClientError> {
        let response = self.send(method, url, bearer, body).await?;
        let status = response.status();
        let body = response.bytes().await?;
        if status != expected {
            return Err(rejected(status, &body));
        }
        serde_json::from_slice(&body).map_err(ClientError::InvalidResponse)
    }

    async fn send(
        &self,
        method: Method,
        url: &url::Url,
        bearer: &str,
        body: Option<Vec<u8>>,
    ) -> Result<reqwest::Response, ClientError> {
        let mut request = self
            .http
            .request(method, url.clone())
            .header(AUTHORIZATION, format!("Bearer {bearer}"))
            .header(ACCEPT, "application/json")
            .timeout(self.request_timeout);
        if let Some(body) = body {
            request = request.header(CONTENT_TYPE, "application/json").body(body);
        }
        Ok(request.send().await?)
    }

    /// The Gateway registers the metadata unchanged or rejects it.
    fn admit(
        &self,
        information: ClientInformation,
        previous_database: Option<DatabaseGrant>,
    ) -> Result<Registration, ClientError> {
        information.check()?;
        if information.metadata != self.metadata {
            return Err(ClientError::ResponseMismatch(
                "registered other metadata than the server sent",
            ));
        }
        let database = information.database.clone().or(previous_database);
        Ok(Registration {
            information,
            database,
        })
    }
}

fn rejected(status: StatusCode, body: &[u8]) -> ClientError {
    ClientError::Rejected {
        status,
        error: serde_json::from_slice(body).ok(),
    }
}
