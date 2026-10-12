//! The OAuth 2.0 Dynamic Client Registration profile that carries the declarations.
//! Standard members keep their RFC 7591 and RFC 7592 names; Veoveo members use the
//! collision-resistant `ai.veoveo/` names. Following RFC 7591, a receiver ignores
//! client metadata it does not understand, while each Veoveo member admits only its
//! own declared fields.
use std::fmt;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use veoveo_types::{OAuthClientId, ServerSlug, TokenIssuer};

use crate::{
    AuthorizationDeclaration, DeclarationError, Ed25519JwkSet, ModuleDeclaration,
    RegistrationDeclaration, ServerKeySet, ServerPublicKey, registration::http_url,
};

/// RFC 7591 `token_endpoint_auth_method`. A server authenticates with RFC 7523 JWTs
/// signed by the key in its `jwks`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, veoveo_types::Vocabulary)]
pub enum TokenEndpointAuthMethod {
    #[vocabulary(rename = "private_key_jwt")]
    PrivateKeyJwt,
}

/// RFC 7591 §2 grant types. This profile registers servers without grants, so the
/// list is empty until a server needs the token endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, veoveo_types::Vocabulary)]
pub enum GrantType {
    AuthorizationCode,
    Implicit,
    Password,
    ClientCredentials,
    RefreshToken,
    #[vocabulary(rename = "urn:ietf:params:oauth:grant-type:jwt-bearer")]
    JwtBearer,
    #[vocabulary(rename = "urn:ietf:params:oauth:grant-type:saml2-bearer")]
    Saml2Bearer,
}

/// RFC 7591 §2 response types. A server uses no authorization endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, veoveo_types::Vocabulary)]
pub enum ResponseType {
    Code,
    Token,
}

/// The client metadata a server registers (RFC 7591 §2) and the Gateway returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClientMetadata {
    pub jwks: ServerKeySet,
    pub token_endpoint_auth_method: TokenEndpointAuthMethod,
    pub grant_types: Vec<GrantType>,
    pub response_types: Vec<ResponseType>,
    #[serde(rename = "ai.veoveo/registration")]
    pub registration: RegistrationDeclaration,
    #[serde(rename = "ai.veoveo/authorization")]
    pub authorization: AuthorizationDeclaration,
    /// Absent for a server without its own tables.
    #[serde(
        rename = "ai.veoveo/module",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub module: Option<ModuleDeclaration>,
}

impl ClientMetadata {
    /// Metadata for a server identified by `key`, checked as the Gateway checks it.
    pub fn new(
        key: ServerPublicKey,
        registration: RegistrationDeclaration,
        authorization: AuthorizationDeclaration,
        module: Option<ModuleDeclaration>,
    ) -> Result<Self, DeclarationError> {
        let metadata = Self {
            jwks: ServerKeySet::new(key),
            token_endpoint_auth_method: TokenEndpointAuthMethod::PrivateKeyJwt,
            grant_types: Vec::new(),
            response_types: Vec::new(),
            registration,
            authorization,
            module,
        };
        metadata.check()?;
        Ok(metadata)
    }

    /// Check the profile and every declaration against the registering namespace.
    pub fn check(&self) -> Result<(), DeclarationError> {
        if !self.grant_types.is_empty() || !self.response_types.is_empty() {
            return Err(DeclarationError::invalid(
                "grant_types",
                "servers register without grant or response types",
            ));
        }
        let slug = &self.registration.slug;
        self.authorization
            .check(slug, &self.registration.uri_scheme)?;
        if let Some(module) = &self.module {
            module.check(slug)?;
        }
        Ok(())
    }

    pub fn slug(&self) -> &ServerSlug {
        &self.registration.slug
    }

    pub fn key(&self) -> &ServerPublicKey {
        self.jwks.key()
    }
}

/// A JWT in compact serialization: three base64url segments.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
pub struct SoftwareStatement(String);

impl SoftwareStatement {
    pub fn new(value: impl Into<String>) -> Result<Self, DeclarationError> {
        let value = value.into();
        let segments = value.split('.').collect::<Vec<_>>();
        if segments.len() != 3
            || segments.iter().any(|segment| {
                segment.is_empty()
                    || !segment
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            })
        {
            return Err(DeclarationError::invalid(
                "software_statement",
                "must be a compact JWT",
            ));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for SoftwareStatement {
    type Error = DeclarationError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<SoftwareStatement> for String {
    fn from(value: SoftwareStatement) -> Self {
        value.0
    }
}
impl fmt::Debug for SoftwareStatement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SoftwareStatement(..)")
    }
}

/// RFC 7591 §3.1: `POST` to the registration endpoint with the installation's
/// registration secret as the initial access token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClientRegistrationRequest {
    #[serde(flatten)]
    pub metadata: ClientMetadata,
    /// Signed by the key in `jwks`; see [`crate::statement`].
    pub software_statement: SoftwareStatement,
}

/// RFC 7592 §2.2: `PUT` to the client configuration endpoint. Renewing a lease is an
/// ordinary update that repeats the registered metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClientUpdateRequest {
    pub client_id: OAuthClientId,
    #[serde(flatten)]
    pub metadata: ClientMetadata,
    pub software_statement: SoftwareStatement,
}

/// RFC 7591 §3.2.1 and RFC 7592 §3: the registered client, returned by registration,
/// read and update.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClientInformation {
    pub client_id: OAuthClientId,
    #[serde(with = "chrono::serde::ts_seconds")]
    #[schemars(with = "i64")]
    pub client_id_issued_at: DateTime<Utc>,
    pub registration_access_token: RegistrationAccessToken,
    #[schemars(with = "url::Url")]
    pub registration_client_uri: url::Url,
    #[serde(flatten)]
    pub metadata: ClientMetadata,
    pub software_statement: SoftwareStatement,
    #[serde(rename = "ai.veoveo/lease")]
    pub lease: RegistrationLease,
    #[serde(rename = "ai.veoveo/access-tokens")]
    pub access_tokens: AccessTokenTrust,
    /// The server's own database user. Present when the server registered a module.
    #[serde(
        rename = "ai.veoveo/database",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub database: Option<DatabaseGrant>,
}

impl ClientInformation {
    /// Check the URLs a client will call with its registration access token.
    pub fn check(&self) -> Result<(), DeclarationError> {
        http_url(&self.registration_client_uri, "registration_client_uri")?;
        self.metadata.check()
    }
}

/// RFC 7592 registration access token. Debug output never shows it.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct RegistrationAccessToken(
    #[serde(serialize_with = "expose")]
    #[schemars(with = "String")]
    SecretString,
);

impl RegistrationAccessToken {
    pub fn new(value: impl Into<String>) -> Self {
        Self(SecretString::from(value.into()))
    }
    pub fn expose_secret(&self) -> &str {
        self.0.expose_secret()
    }
}
impl fmt::Debug for RegistrationAccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistrationAccessToken([REDACTED])")
    }
}

fn expose<S: serde::Serializer>(secret: &SecretString, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(secret.expose_secret())
}

/// `ai.veoveo/lease`: when the registration expires unless the server updates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistrationLease {
    pub expires_at: DateTime<Utc>,
    /// Send an RFC 7592 update at this interval.
    pub renew_every_seconds: u32,
}

/// `ai.veoveo/access-tokens`: the issuer and keys of the RFC 9068 access tokens the
/// Gateway forwards to this server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccessTokenTrust {
    pub issuer: TokenIssuer,
    pub jwks: Ed25519JwkSet,
}

/// `ai.veoveo/database`: a database user the Gateway created for one server.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DatabaseGrant {
    #[schemars(with = "url::Url")]
    pub endpoint: url::Url,
    pub namespace: DatabaseIdentifier,
    pub database: DatabaseIdentifier,
    pub username: DatabaseIdentifier,
    #[serde(serialize_with = "expose")]
    #[schemars(with = "String")]
    pub password: SecretString,
}

impl fmt::Debug for DatabaseGrant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatabaseGrant")
            .field("endpoint", &self.endpoint.as_str())
            .field("namespace", &self.namespace)
            .field("database", &self.database)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

/// A SurrealDB namespace, database or user name: ASCII letters, digits and underscore.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
pub struct DatabaseIdentifier(String);

impl DatabaseIdentifier {
    pub fn new(value: impl Into<String>) -> Result<Self, DeclarationError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(DeclarationError::invalid(
                "database identifier",
                "1..=128 ASCII letters, digits or underscore",
            ));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for DatabaseIdentifier {
    type Error = DeclarationError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<DatabaseIdentifier> for String {
    fn from(value: DatabaseIdentifier) -> Self {
        value.0
    }
}

/// RFC 7591 §3.2.2 error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, veoveo_types::Vocabulary)]
pub enum RegistrationErrorCode {
    InvalidRedirectUri,
    /// A declaration fails its checks or leaves the server's namespace.
    InvalidClientMetadata,
    /// The software statement is malformed, unsigned by the `jwks` key, stale, or
    /// disagrees with the request metadata.
    InvalidSoftwareStatement,
    /// The namespace is pinned to another key.
    UnapprovedSoftwareStatement,
}

/// RFC 7591 §3.2.2 error response, sent with HTTP 400.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RegistrationErrorResponse {
    pub error: RegistrationErrorCode,
    /// The exact reason, naming declared identifiers only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

/// The RFC 8414 authorization server metadata members a registering server reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AuthorizationServerMetadata {
    #[schemars(with = "url::Url")]
    pub issuer: url::Url,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<url::Url>")]
    pub registration_endpoint: Option<url::Url>,
}

impl AuthorizationServerMetadata {
    /// RFC 8414 §3.1: the well-known URL for `issuer`, inserted before any issuer path.
    pub fn url_for(issuer: &url::Url) -> url::Url {
        let mut url = issuer.clone();
        let path = issuer.path().trim_end_matches('/');
        url.set_path(&format!("/.well-known/oauth-authorization-server{path}"));
        url.set_query(None);
        url.set_fragment(None);
        url
    }
}
