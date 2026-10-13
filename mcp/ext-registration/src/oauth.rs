//! The OAuth 2.0 Dynamic Client Registration profile that carries the declarations.
//! Standard members keep their RFC 7591 and RFC 7592 names: the slug is `software_id`
//! and the build `software_version`. Members without a standard equivalent take the
//! RFC 6749 §8.2 vendor prefix in snake_case, such as `veoveo_authorization`, and times
//! are NumericDate seconds. Following RFC 7591, a receiver ignores client metadata it
//! does not understand, while each Veoveo member admits only its own declared fields.
use std::fmt;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use veoveo_types::{OAuthClientId, ResourceScheme, ServerSlug, TokenIssuer};

use crate::{
    AuthorizationDeclaration, DeclarationError, Ed25519JwkSet, RegistrationDeclaration,
    ResourceIndicator, ServerEndpoint, ServerKeySet, ServerPublicKey, ServerVersion,
    authorization::AuthorizationDocument, registration::http_url,
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
#[serde(from = "ClientMetadataWire", into = "ClientMetadataWire")]
#[schemars(with = "ClientMetadataWire")]
pub struct ClientMetadata {
    pub jwks: ServerKeySet,
    pub token_endpoint_auth_method: TokenEndpointAuthMethod,
    pub grant_types: Vec<GrantType>,
    pub response_types: Vec<ResponseType>,
    pub registration: RegistrationDeclaration,
    pub authorization: AuthorizationDeclaration,
}

/// The members of [`ClientMetadata`] as RFC 7591 names them.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "ClientMetadata")]
struct ClientMetadataWire {
    jwks: ServerKeySet,
    token_endpoint_auth_method: TokenEndpointAuthMethod,
    grant_types: Vec<GrantType>,
    response_types: Vec<ResponseType>,
    /// The server's slug.
    software_id: ServerSlug,
    software_version: ServerVersion,
    veoveo_uri_scheme: ResourceScheme,
    veoveo_mcp_endpoint: ServerEndpoint,
    veoveo_authorization: AuthorizationDocument,
}

impl From<ClientMetadataWire> for ClientMetadata {
    fn from(wire: ClientMetadataWire) -> Self {
        Self {
            jwks: wire.jwks,
            token_endpoint_auth_method: wire.token_endpoint_auth_method,
            grant_types: wire.grant_types,
            response_types: wire.response_types,
            registration: RegistrationDeclaration {
                slug: wire.software_id,
                uri_scheme: wire.veoveo_uri_scheme,
                version: wire.software_version,
                endpoint: wire.veoveo_mcp_endpoint,
            },
            authorization: wire.veoveo_authorization.into(),
        }
    }
}
impl From<ClientMetadata> for ClientMetadataWire {
    fn from(metadata: ClientMetadata) -> Self {
        Self {
            jwks: metadata.jwks,
            token_endpoint_auth_method: metadata.token_endpoint_auth_method,
            grant_types: metadata.grant_types,
            response_types: metadata.response_types,
            software_id: metadata.registration.slug,
            software_version: metadata.registration.version,
            veoveo_uri_scheme: metadata.registration.uri_scheme,
            veoveo_mcp_endpoint: metadata.registration.endpoint,
            veoveo_authorization: metadata.authorization.into(),
        }
    }
}

impl ClientMetadata {
    /// Metadata for a server identified by `key`, checked as the Gateway checks it.
    pub fn new(
        key: ServerPublicKey,
        registration: RegistrationDeclaration,
        authorization: AuthorizationDeclaration,
    ) -> Result<Self, DeclarationError> {
        let metadata = Self {
            jwks: ServerKeySet::new(key),
            token_endpoint_auth_method: TokenEndpointAuthMethod::PrivateKeyJwt,
            grant_types: Vec::new(),
            response_types: Vec::new(),
            registration,
            authorization,
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
        if slug.is_reserved() {
            return Err(DeclarationError::invalid(
                "software_id",
                "the slug is reserved for the platform, a database schema or a registered URI scheme",
            ));
        }
        if self.registration.uri_scheme.as_ref() != slug.as_ref() {
            return Err(DeclarationError::invalid(
                "veoveo_uri_scheme",
                "a server's URI scheme equals its slug",
            ));
        }
        self.authorization
            .check(slug, &self.registration.uri_scheme)
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

/// RFC 7592 §2.2: `PUT` to the client configuration endpoint. It replaces the registered
/// metadata.
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
    #[serde(rename = "veoveo_access_tokens")]
    pub access_tokens: AccessTokenTrust,
    /// The server's own PostgreSQL login, present when the installation grants one.
    #[serde(
        rename = "veoveo_database",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub database: Option<DatabaseGrant>,
}

impl ClientInformation {
    /// Check the URLs a client will call with its registration access token, the
    /// access-token issuer and audience, and the registered metadata.
    pub fn check(&self) -> Result<(), DeclarationError> {
        http_url(&self.registration_client_uri, "registration_client_uri")?;
        self.access_tokens.check()?;
        if self.access_tokens.resource
            != ResourceIndicator::from(&self.metadata.registration.endpoint)
        {
            return Err(DeclarationError::invalid(
                "veoveo_access_tokens",
                "the token audience must be the registered MCP endpoint",
            ));
        }
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

/// `veoveo_access_tokens`: the issuer, keys and audience of the RFC 9068 access tokens
/// the Gateway forwards to this server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AccessTokenTrust {
    /// The Gateway's HTTPS issuer URL, as its RFC 8414 metadata announces it.
    pub issuer: TokenIssuer,
    pub jwks: Ed25519JwkSet,
    /// The `aud` of every token addressed to this server: its registered MCP endpoint.
    pub resource: ResourceIndicator,
}

impl AccessTokenTrust {
    /// The issuer is an HTTPS URL without credentials, query or fragment.
    pub fn check(&self) -> Result<(), DeclarationError> {
        issuer_url(&self.issuer)
    }
}

/// An RFC 8414 issuer: an HTTPS URL, or an HTTP URL on a local host, without
/// credentials, query or fragment.
pub fn issuer_url(issuer: &TokenIssuer) -> Result<(), DeclarationError> {
    let url = url::Url::parse(issuer.as_str())
        .map_err(|_| DeclarationError::invalid("issuer", "must be a URL"))?;
    http_url(&url, "issuer")
}

/// `veoveo_database`: the PostgreSQL login the Gateway created for one registration.
/// The login is a member of the server's group role, which owns the server's schema.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DatabaseGrant {
    /// The installation database, without credentials.
    pub url: DatabaseUrl,
    pub username: DatabaseIdentifier,
    #[serde(serialize_with = "expose")]
    #[schemars(with = "String")]
    pub password: SecretString,
}

impl fmt::Debug for DatabaseGrant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatabaseGrant")
            .field("url", &self.url.as_str())
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

/// A `postgres://` or `postgresql://` URL that names a host and one database. It carries
/// no user, password or fragment; query parameters such as `sslmode` are connection
/// options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "url::Url")]
pub struct DatabaseUrl(url::Url);

impl DatabaseUrl {
    pub fn new(url: url::Url) -> Result<Self, DeclarationError> {
        if !matches!(url.scheme(), "postgres" | "postgresql") {
            return Err(DeclarationError::invalid(
                "veoveo_database",
                "the URL scheme must be postgres or postgresql",
            ));
        }
        if url.host_str().is_none_or(str::is_empty) {
            return Err(DeclarationError::invalid(
                "veoveo_database",
                "the URL must name a host",
            ));
        }
        if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
            return Err(DeclarationError::invalid(
                "veoveo_database",
                "the URL carries no user, password or fragment",
            ));
        }
        let database = url.path().strip_prefix('/').unwrap_or_default();
        if database.is_empty() || database.contains('/') {
            return Err(DeclarationError::invalid(
                "veoveo_database",
                "the URL path names exactly one database",
            ));
        }
        Ok(Self(url))
    }
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
    pub fn as_url(&self) -> &url::Url {
        &self.0
    }
}
impl TryFrom<String> for DatabaseUrl {
    type Error = DeclarationError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let url = url::Url::parse(&value)
            .map_err(|_| DeclarationError::invalid("veoveo_database", "must be a URL"))?;
        Self::new(url)
    }
}
impl From<DatabaseUrl> for String {
    fn from(value: DatabaseUrl) -> Self {
        value.0.into()
    }
}

/// A PostgreSQL role name: a lowercase letter or underscore followed by up to 62
/// lowercase letters, digits or underscores, so it fits the 63-byte identifier limit
/// without quoting.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
pub struct DatabaseIdentifier(#[schemars(regex(pattern = r"^[a-z_][a-z0-9_]{0,62}$"))] String);

impl DatabaseIdentifier {
    pub fn new(value: impl Into<String>) -> Result<Self, DeclarationError> {
        let value = value.into();
        let mut bytes = value.bytes();
        let admitted = bytes
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first == b'_')
            && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            && value.len() <= 63;
        if !admitted {
            return Err(DeclarationError::invalid(
                "database identifier",
                "a lowercase letter or underscore, then up to 62 lowercase letters, digits or underscores",
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

/// The RFC 8414 authorization server metadata members a server reads: the registration
/// endpoint when it registers, and the JWK Set when it refreshes the keys that verify
/// forwarded access tokens (RFC 9068 §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AuthorizationServerMetadata {
    #[schemars(with = "url::Url")]
    pub issuer: url::Url,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<url::Url>")]
    pub registration_endpoint: Option<url::Url>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<url::Url>")]
    pub jwks_uri: Option<url::Url>,
}

impl AuthorizationServerMetadata {
    /// The `jwks_uri` member: an HTTPS URL, or an HTTP URL on a local host, without
    /// credentials, query or fragment.
    pub fn key_set_url(&self) -> Result<&url::Url, DeclarationError> {
        let url = self.jwks_uri.as_ref().ok_or(DeclarationError::invalid(
            "jwks_uri",
            "the metadata names no JWK Set",
        ))?;
        http_url(url, "jwks_uri")?;
        Ok(url)
    }

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
