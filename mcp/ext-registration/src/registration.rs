//! `ai.veoveo/registration`: who the server is and where the Gateway reaches it.
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use veoveo_types::{ResourceScheme, ServerSlug};

use crate::DeclarationError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistrationDeclaration {
    pub slug: ServerSlug,
    pub uri_scheme: ResourceScheme,
    pub version: ServerVersion,
    pub endpoint: ServerEndpoint,
}

/// A server build's version label, such as `0.1.0` or a Git revision.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
pub struct ServerVersion(String);

impl ServerVersion {
    pub fn new(value: impl Into<String>) -> Result<Self, DeclarationError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
        {
            return Err(DeclarationError::invalid(
                "version",
                "1..=64 ASCII letters, digits, dot, underscore, plus or hyphen",
            ));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for ServerVersion {
    type Error = DeclarationError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<ServerVersion> for String {
    fn from(value: ServerVersion) -> Self {
        value.0
    }
}
impl fmt::Display for ServerVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The URL where the Gateway reaches the server's MCP endpoint. HTTP or HTTPS, with a
/// host and without credentials, query or fragment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "url::Url", into = "url::Url")]
#[schemars(with = "url::Url")]
pub struct ServerEndpoint(url::Url);

impl ServerEndpoint {
    pub fn new(url: url::Url) -> Result<Self, DeclarationError> {
        http_url(&url, "endpoint")?;
        Ok(Self(url))
    }
    pub fn url(&self) -> &url::Url {
        &self.0
    }
}
impl TryFrom<url::Url> for ServerEndpoint {
    type Error = DeclarationError;
    fn try_from(value: url::Url) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<ServerEndpoint> for url::Url {
    fn from(value: ServerEndpoint) -> Self {
        value.0
    }
}

/// An HTTP or HTTPS URL with a host and without credentials, query or fragment.
pub(crate) fn http_url(url: &url::Url, field: &'static str) -> Result<(), DeclarationError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(DeclarationError::invalid(field, "must use http or https"));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(DeclarationError::invalid(field, "must name a host"));
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(DeclarationError::invalid(
            field,
            "must not carry credentials, a query or a fragment",
        ));
    }
    Ok(())
}

/// An RFC 8707 resource indicator: the absolute URL a token's `aud` names. HTTP or
/// HTTPS, with a host and without credentials, query or fragment.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "url::Url", into = "url::Url")]
#[schemars(with = "url::Url")]
pub struct ResourceIndicator(url::Url);

impl ResourceIndicator {
    pub fn new(url: url::Url) -> Result<Self, DeclarationError> {
        http_url(&url, "resource")?;
        Ok(Self(url))
    }
    pub fn parse(value: &str) -> Result<Self, DeclarationError> {
        Self::new(
            url::Url::parse(value)
                .map_err(|_| DeclarationError::invalid("resource", "must be a URL"))?,
        )
    }
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}
impl TryFrom<url::Url> for ResourceIndicator {
    type Error = DeclarationError;
    fn try_from(value: url::Url) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<ResourceIndicator> for url::Url {
    fn from(value: ResourceIndicator) -> Self {
        value.0
    }
}
impl From<&ServerEndpoint> for ResourceIndicator {
    fn from(endpoint: &ServerEndpoint) -> Self {
        Self(endpoint.0.clone())
    }
}
impl fmt::Display for ResourceIndicator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}
