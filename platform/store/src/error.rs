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
    #[error("the process instance ended: it shut down, or the database refused its login")]
    InstanceLost,
    #[error("invalid platform identity field {field}: {reason}")]
    InvalidIdentityField {
        field: &'static str,
        reason: &'static str,
    },
    #[error("existing {entity} identity conflicts with canonical key {key}")]
    IdentityConflict { entity: &'static str, key: String },
    #[error("the stored {column} is not an admitted value")]
    InvalidStoredValue { column: &'static str },
    #[error("invalid refresh-token transition: {reason}")]
    InvalidRefreshTransition { reason: &'static str },
    #[error("another control-plane revision became active first")]
    ControlRevisionChanged,
    #[error("the control-plane revision already exists")]
    ControlRevisionExists,
    #[error("records appended with the change failed: {0}")]
    Append(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("a maintenance batch holds 1 to 10000 rows, not {0}")]
    InvalidBatch(u16),
}

impl StoreError {
    /// The error reports a connection that ended or could not be had, which a later
    /// attempt on a new connection may get past: see [`is_connection_error`]. A deadline
    /// on opening a connection counts too.
    pub fn is_connection(&self) -> bool {
        match self {
            Self::Database(error) => is_connection_error(error),
            Self::Timeout { .. } => true,
            _ => false,
        }
    }

    /// The database refused the login: its password no longer matches, or the role is
    /// gone or may not log in. A registered server's login ends this way once the
    /// Gateway removed the server's registration.
    pub fn is_login_refused(&self) -> bool {
        matches!(self, Self::Database(error) if is_login_refused(error))
    }

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

/// The error reports a connection that ended or could not be had, which a later attempt
/// on a new connection may get past: an I/O failure, a session the server terminated or
/// is shutting down (SQLSTATE class 08, `57P01` to `57P03`), a server out of connections
/// (`53300`), or a pool that could not hand out a connection in time.
pub fn is_connection_error(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Io(_) | sqlx::Error::Tls(_) | sqlx::Error::PoolTimedOut => true,
        sqlx::Error::Database(error) => error.code().is_some_and(|code| {
            code.starts_with("08") || matches!(code.as_ref(), "57P01" | "57P02" | "57P03" | "53300")
        }),
        _ => false,
    }
}

/// The database refused the login (SQLSTATE `28000` or `28P01`).
fn is_login_refused(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(error)
            if error.code().is_some_and(|code| matches!(code.as_ref(), "28000" | "28P01"))
    )
}
