use thiserror::Error;

/// A rejected [`StoreConfig`](crate::StoreConfig). Messages never repeat the URL or the
/// password.
#[derive(Debug, Error, Clone, Eq, PartialEq)]
pub enum StoreConfigError {
    #[error("the database URL is not a valid postgres:// URL")]
    InvalidUrl,
    #[error("the database URL scheme must be postgres or postgresql")]
    UnsupportedScheme,
    #[error("the database URL must name a host")]
    MissingHost,
    #[error("the database URL path must name exactly one database")]
    MissingDatabase,
    #[error("the database URL must not carry a password or fragment")]
    UnsafeUrl,
    #[error("the database user is missing from both the URL and the configuration")]
    MissingUsername,
    #[error("the database password must not be empty")]
    EmptyPassword,
    #[error("{field} must be greater than zero")]
    ZeroValue { field: &'static str },
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error(transparent)]
    Config(#[from] StoreConfigError),
    #[error("database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("database migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("{operation} exceeded {seconds} seconds")]
    Timeout {
        operation: &'static str,
        seconds: u64,
    },
    #[error("invalid {kind} {name:?}: {reason}")]
    InvalidName {
        kind: &'static str,
        name: String,
        reason: &'static str,
    },
    #[error("the process instance lost its database connection")]
    InstanceLost,
    #[error("invalid platform identity field {field}: {reason}")]
    InvalidIdentityField {
        field: &'static str,
        reason: &'static str,
    },
    #[error("existing {entity} identity conflicts with canonical key {key}")]
    IdentityConflict { entity: &'static str, key: String },
}

impl StoreError {
    pub(crate) fn invalid_name(
        kind: &'static str,
        name: impl Into<String>,
        reason: &'static str,
    ) -> Self {
        Self::InvalidName {
            kind,
            name: name.into(),
            reason,
        }
    }
}
