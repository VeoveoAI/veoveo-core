//! Gateway profiles and the URL and path types installation configuration uses. A
//! profile names no server: registered servers become visible through the
//! permissions their declarations grant.
use std::{collections::BTreeSet, fmt, str::FromStr};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct GatewayProfile {
    pub id: GatewayProfileId,
    pub identity_provider: IdentityProviderId,
    pub authorization_server: AuthorizationServerId,
    pub protected_resource: ProtectedResourceId,
    pub policy_version: PolicyVersion,
    /// An explicit policy enables the public resumable upload surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_upload: Option<veoveo_artifact_contract::ArtifactUploadPolicy>,
    pub auth_modes: BTreeSet<AuthMode>,
    /// Whether federated list discovery may return a typed degraded catalog or
    /// must fail until every registered server is reachable.
    #[serde(default)]
    pub discovery_failure_mode: DiscoveryFailureMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_scopes: Vec<ScopeName>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryFailureMode {
    #[default]
    Isolate,
    FailClosed,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
pub struct HttpsUrl(String);

impl HttpsUrl {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        validate_https_url(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for HttpsUrl {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for HttpsUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for HttpsUrl {
    type Error = IdentifierError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<HttpsUrl> for String {
    fn from(value: HttpsUrl) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
pub struct OAuthEndpointUrl(String);

impl OAuthEndpointUrl {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        validate_oauth_endpoint_url(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for OAuthEndpointUrl {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for OAuthEndpointUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for OAuthEndpointUrl {
    type Error = IdentifierError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<OAuthEndpointUrl> for String {
    fn from(value: OAuthEndpointUrl) -> Self {
        value.0
    }
}

pub use veoveo_gateway_contract::OAuthRedirectUri;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
pub struct JwksFilePath(String);

impl JwksFilePath {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        validate_local_file_path(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for JwksFilePath {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for JwksFilePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for JwksFilePath {
    type Error = IdentifierError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl FromStr for JwksFilePath {
    type Err = IdentifierError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value.to_string())
    }
}

impl From<JwksFilePath> for String {
    fn from(value: JwksFilePath) -> Self {
        value.0
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    OidcAuthorizationCodePkce,
    EnterpriseManagedAuthorization,
    #[serde(rename = "oauth_client_credentials")]
    OAuthClientCredentials,
}

impl AuthMode {
    pub fn mcp_extension_id(self) -> Option<&'static str> {
        match self {
            Self::OidcAuthorizationCodePkce => None,
            Self::EnterpriseManagedAuthorization => {
                Some(MCP_ENTERPRISE_MANAGED_AUTHORIZATION_EXTENSION)
            }
            Self::OAuthClientCredentials => Some(MCP_OAUTH_CLIENT_CREDENTIALS_EXTENSION),
        }
    }
}

impl From<AuthMode> for OAuthGrantType {
    fn from(value: AuthMode) -> Self {
        match value {
            AuthMode::OidcAuthorizationCodePkce => Self::AuthorizationCodePkce,
            AuthMode::EnterpriseManagedAuthorization => Self::EnterpriseManagedAuthorization,
            AuthMode::OAuthClientCredentials => Self::ClientCredentials,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_endpoints_allow_only_https_or_explicit_local_http() {
        OAuthEndpointUrl::new("https://idp.example.com/oauth/token").unwrap();
        OAuthEndpointUrl::new("http://localhost:8780/oauth/token").unwrap();
        OAuthEndpointUrl::new("http://127.0.0.1:8780/oauth/token").unwrap();
        OAuthEndpointUrl::new("http://host.openshell.internal:8080/oauth/token").unwrap();

        assert!(OAuthEndpointUrl::new("http://localhost/oauth/token").is_err());
        assert!(OAuthEndpointUrl::new("http://gateway:8780/oauth/token").is_err());
    }
}
