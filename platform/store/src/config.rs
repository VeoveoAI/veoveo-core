//! Connection configuration: a `postgres://` URL without a password, the password as a
//! secret, and the pool's size and timeouts.
use std::{fmt, str::FromStr, time::Duration};

use secrecy::{ExposeSecret, SecretString};
use sqlx::postgres::PgConnectOptions;
use url::Url;

use crate::{INSTANCE_GRACE, InstanceGrace, StoreConfigError};

const DEFAULT_MAX_CONNECTIONS: u32 = 10;
const DEFAULT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const DEFAULT_MAX_LIFETIME: Duration = Duration::from_secs(30 * 60);

/// Validated connection settings for one database. Debug output never shows the password.
#[derive(Clone)]
pub struct StoreConfig {
    url: Url,
    username: String,
    database: String,
    password: SecretString,
    max_connections: u32,
    acquire_timeout: Duration,
    idle_timeout: Option<Duration>,
    max_lifetime: Option<Duration>,
    application_name: Option<String>,
    instance_grace: InstanceGrace,
}

impl StoreConfig {
    /// `url` names the host and the database and may name the user; it carries no
    /// password. Query parameters such as `sslmode` are connection options.
    pub fn builder(url: impl Into<String>, password: SecretString) -> StoreConfigBuilder {
        StoreConfigBuilder {
            url: url.into(),
            password,
            username: None,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            acquire_timeout: DEFAULT_ACQUIRE_TIMEOUT,
            idle_timeout: Some(DEFAULT_IDLE_TIMEOUT),
            max_lifetime: Some(DEFAULT_MAX_LIFETIME),
            application_name: None,
            instance_grace: INSTANCE_GRACE,
        }
    }

    /// The database URL without credentials.
    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn database(&self) -> &str {
        &self.database
    }

    pub fn password(&self) -> &SecretString {
        &self.password
    }

    pub fn max_connections(&self) -> u32 {
        self.max_connections
    }

    pub fn acquire_timeout(&self) -> Duration {
        self.acquire_timeout
    }

    pub fn idle_timeout(&self) -> Option<Duration> {
        self.idle_timeout
    }

    pub fn max_lifetime(&self) -> Option<Duration> {
        self.max_lifetime
    }

    /// How long an instance's lock stays free before this process's reaper deletes the
    /// instance and moves its work to recovery. Every process of an installation should
    /// use the same grace; the shortest one any process uses decides.
    pub fn instance_grace(&self) -> InstanceGrace {
        self.instance_grace
    }

    /// The same settings with another instance grace.
    pub fn with_instance_grace(&self, grace: InstanceGrace) -> Self {
        Self {
            instance_grace: grace,
            ..self.clone()
        }
    }

    /// The same database and pool settings for another login.
    pub fn with_login(&self, username: &str, password: SecretString) -> Self {
        Self {
            username: username.to_owned(),
            password,
            ..self.clone()
        }
    }

    pub(crate) fn connect_options(&self) -> PgConnectOptions {
        let options = PgConnectOptions::from_str(self.url.as_str())
            .expect("the URL was admitted by StoreConfigBuilder::build")
            .username(&self.username)
            .password(self.password.expose_secret());
        match &self.application_name {
            Some(name) => options.application_name(name),
            None => options,
        }
    }
}

impl fmt::Debug for StoreConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoreConfig")
            .field("url", &self.url.as_str())
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("max_connections", &self.max_connections)
            .field("acquire_timeout", &self.acquire_timeout)
            .field("idle_timeout", &self.idle_timeout)
            .field("max_lifetime", &self.max_lifetime)
            .field("application_name", &self.application_name)
            .field("instance_grace", &self.instance_grace)
            .finish()
    }
}

pub struct StoreConfigBuilder {
    url: String,
    password: SecretString,
    username: Option<String>,
    max_connections: u32,
    acquire_timeout: Duration,
    idle_timeout: Option<Duration>,
    max_lifetime: Option<Duration>,
    application_name: Option<String>,
    instance_grace: InstanceGrace,
}

impl StoreConfigBuilder {
    /// The login, which replaces a user named in the URL.
    pub fn username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    pub fn max_connections(mut self, value: u32) -> Self {
        self.max_connections = value;
        self
    }

    /// How long a query waits for a pooled connection, including opening one.
    pub fn acquire_timeout(mut self, value: Duration) -> Self {
        self.acquire_timeout = value;
        self
    }

    pub fn idle_timeout(mut self, value: Option<Duration>) -> Self {
        self.idle_timeout = value;
        self
    }

    pub fn max_lifetime(mut self, value: Option<Duration>) -> Self {
        self.max_lifetime = value;
        self
    }

    /// The `application_name` PostgreSQL shows for this process's sessions.
    pub fn application_name(mut self, value: impl Into<String>) -> Self {
        self.application_name = Some(value.into());
        self
    }

    /// How long an instance's lock stays free before this process's reaper deletes the
    /// instance, [`INSTANCE_GRACE`] by default. [`InstanceGrace::ZERO`] deletes an
    /// instance as soon as its lock is free.
    pub fn instance_grace(mut self, value: InstanceGrace) -> Self {
        self.instance_grace = value;
        self
    }

    pub fn build(self) -> Result<StoreConfig, StoreConfigError> {
        let url = Url::parse(&self.url).map_err(|_| StoreConfigError::InvalidUrl)?;
        if !matches!(url.scheme(), "postgres" | "postgresql") {
            return Err(StoreConfigError::UnsupportedScheme);
        }
        if url.host_str().is_none_or(str::is_empty) {
            return Err(StoreConfigError::MissingHost);
        }
        if url.password().is_some() || url.fragment().is_some() {
            return Err(StoreConfigError::UnsafeUrl);
        }
        let database = url.path().strip_prefix('/').unwrap_or_default();
        if database.is_empty() || database.contains('/') {
            return Err(StoreConfigError::MissingDatabase);
        }
        let database = percent_encoding::percent_decode_str(database)
            .decode_utf8()
            .map_err(|_| StoreConfigError::InvalidUrl)?
            .into_owned();
        PgConnectOptions::from_str(url.as_str()).map_err(|_| StoreConfigError::InvalidUrl)?;
        let username = match self.username {
            Some(username) => username,
            None => percent_encoding::percent_decode_str(url.username())
                .decode_utf8()
                .map_err(|_| StoreConfigError::InvalidUrl)?
                .into_owned(),
        };
        if username.is_empty() {
            return Err(StoreConfigError::MissingUsername);
        }
        if self.password.expose_secret().is_empty() {
            return Err(StoreConfigError::EmptyPassword);
        }
        if self.max_connections == 0 {
            return Err(StoreConfigError::ZeroValue {
                field: "max_connections",
            });
        }
        if self.acquire_timeout.is_zero() {
            return Err(StoreConfigError::ZeroValue {
                field: "acquire_timeout",
            });
        }
        let mut url = url;
        url.set_username("")
            .expect("a URL with a host accepts an empty user");
        Ok(StoreConfig {
            url,
            username,
            database,
            password: self.password,
            max_connections: self.max_connections,
            acquire_timeout: self.acquire_timeout,
            idle_timeout: self.idle_timeout,
            max_lifetime: self.max_lifetime,
            application_name: self.application_name,
            instance_grace: self.instance_grace,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(value: &str) -> SecretString {
        SecretString::from(value.to_owned())
    }

    #[test]
    fn admits_a_url_without_password_and_redacts_the_secret() {
        let config = StoreConfig::builder(
            "postgres://gateway@db.internal.example:5432/veoveo?sslmode=verify-full",
            secret("not-for-logs"),
        )
        .application_name("veoveo-service-gateway")
        .build()
        .unwrap();
        assert_eq!(config.username(), "gateway");
        assert_eq!(config.database(), "veoveo");
        assert_eq!(
            config.url().as_str(),
            "postgres://db.internal.example:5432/veoveo?sslmode=verify-full"
        );
        let debug = format!("{config:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("not-for-logs"));
        let options = config.connect_options();
        assert_eq!(options.get_username(), "gateway");
        assert_eq!(options.get_database(), Some("veoveo"));
    }

    #[test]
    fn a_login_replaces_the_url_user() {
        let config = StoreConfig::builder(
            "postgresql://postgres.internal/veoveo",
            secret("server-password"),
        )
        .username("time_0b6f")
        .build()
        .unwrap();
        assert_eq!(config.username(), "time_0b6f");
        let other = config.with_login("time_1c7a", secret("other-password"));
        assert_eq!(other.username(), "time_1c7a");
        assert_eq!(other.url(), config.url());
    }

    #[test]
    fn rejects_unsafe_or_incomplete_urls_and_limits() {
        for (url, error) in [
            ("not a url", StoreConfigError::InvalidUrl),
            ("mysql://db/veoveo", StoreConfigError::UnsupportedScheme),
            (
                "postgres://user:secret@db/veoveo",
                StoreConfigError::UnsafeUrl,
            ),
            ("postgres://user@db/veoveo#x", StoreConfigError::UnsafeUrl),
            ("postgres://user@db/", StoreConfigError::MissingDatabase),
            ("postgres://user@db/a/b", StoreConfigError::MissingDatabase),
            ("postgres://db/veoveo", StoreConfigError::MissingUsername),
        ] {
            assert_eq!(
                StoreConfig::builder(url, secret("password"))
                    .build()
                    .unwrap_err(),
                error,
                "{url}"
            );
        }
        assert_eq!(
            StoreConfig::builder("postgres://user@db/veoveo", secret(""))
                .build()
                .unwrap_err(),
            StoreConfigError::EmptyPassword
        );
        assert_eq!(
            StoreConfig::builder("postgres://user@db/veoveo", secret("password"))
                .max_connections(0)
                .build()
                .unwrap_err(),
            StoreConfigError::ZeroValue {
                field: "max_connections"
            }
        );
    }
}
