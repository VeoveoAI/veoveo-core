use url::Url;

use super::*;

pub(super) fn validate_https_url(value: &str) -> Result<(), IdentifierError> {
    if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(IdentifierError::new(
            value,
            "must not contain whitespace or control characters",
        ));
    }
    let url = Url::parse(value).map_err(|_| IdentifierError::new(value, "must be a valid URL"))?;
    if url.scheme() != "https" {
        return Err(IdentifierError::new(value, "must use https://"));
    }
    if url.host().is_none() {
        return Err(IdentifierError::new(value, "must include a host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(IdentifierError::new(value, "must not contain userinfo"));
    }
    if url.fragment().is_some() {
        return Err(IdentifierError::new(value, "must not contain a fragment"));
    }
    Ok(())
}

pub(super) fn validate_oauth_endpoint_url(value: &str) -> Result<(), IdentifierError> {
    if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(IdentifierError::new(
            value,
            "must not contain whitespace or control characters",
        ));
    }
    let url = Url::parse(value).map_err(|_| IdentifierError::new(value, "must be a valid URL"))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(IdentifierError::new(value, "must not contain userinfo"));
    }
    if url.fragment().is_some() {
        return Err(IdentifierError::new(value, "must not contain a fragment"));
    }
    match url.scheme() {
        "https" if url.host().is_some() => Ok(()),
        "http" => {
            if veoveo_types::is_local_host(&url) && url.port().is_some_and(|port| port != 0) {
                return Ok(());
            }
            Err(IdentifierError::new(
                value,
                "http:// OAuth endpoints must use a local host and explicit non-zero port",
            ))
        }
        _ => Err(IdentifierError::new(
            value,
            "must use https:// or local loopback http://",
        )),
    }
}

pub(super) fn validate_local_file_path(value: &str) -> Result<(), IdentifierError> {
    if value.is_empty() {
        return Err(IdentifierError::new(value, "must not be empty"));
    }
    if value.starts_with("http://") || value.starts_with("https://") || value.starts_with("file://")
    {
        return Err(IdentifierError::new(
            value,
            "must be a local filesystem path, not a URL",
        ));
    }
    if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(IdentifierError::new(
            value,
            "must not contain whitespace or control characters",
        ));
    }
    Ok(())
}
