//! Task runtime failures.
use veoveo_types::TaskId;

use crate::TaskStatus;

#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("task `{0}` was not found")]
    NotFound(TaskId),
    #[error("task belongs to server `{0}`, not this runtime's server")]
    WrongServer(String),
    #[error("task transition from {from:?} to {to:?} is not allowed")]
    InvalidTransition { from: TaskStatus, to: TaskStatus },
    #[error("task `{0}` changed concurrently")]
    Conflict(TaskId),
    #[error("task `{0}` is not owned by this instance")]
    NotOwner(TaskId),
    #[error("task progress must be finite and in 0..=1")]
    InvalidProgress,
    #[error("invalid task invocation authority: {0}")]
    InvalidAuthority(String),
    #[error("invalid task record: {0}")]
    InvalidRecord(String),
    #[error(
        "task query requires 1..=32 task types, at most 256 subscribed IDs and a limit in 1..=1000"
    )]
    InvalidPageQuery,
    #[error("the page cursor continues a {cursor} page, not a {requested} page")]
    PageOrderMismatch {
        cursor: crate::TaskPageOrder,
        requested: crate::TaskPageOrder,
    },
    #[error("a batch names {requested} Tasks, more than the {max} one read takes")]
    BatchTooLarge { requested: usize, max: usize },
    #[error("task input key is empty, too long, or contains a control character")]
    InvalidInputKey,
    #[error("task input key `{0}` has already been used")]
    DuplicateInputKey(String),
    #[error("task `{0}` already holds 64 input requests, the most a Task holds")]
    InputLimit(TaskId),
    #[error("task `{0}` is parked on an operation and waits without a worker")]
    Parked(TaskId),
    #[error("the tasks notification listener stopped")]
    ListenerStopped,
    #[error(transparent)]
    Store(#[from] veoveo_platform_store::StoreError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl TaskError {
    /// The worker parked its Task on an operation and stops without settling it.
    pub fn is_parked(&self) -> bool {
        matches!(self, Self::Parked(_))
    }

    /// The error reports a database connection that ended or could not be had, so the
    /// same work may succeed on a new connection.
    pub fn is_connection(&self) -> bool {
        match self {
            Self::Store(error) => error.is_connection(),
            Self::Database(error) => veoveo_platform_store::is_connection_error(error),
            _ => false,
        }
    }

    pub(crate) fn invalid_record(message: impl Into<String>) -> Self {
        Self::InvalidRecord(message.into())
    }
}

/// Whether `error` or one of its sources reports a database connection that ended or
/// could not be had. A worker uses it to tell work that a lost connection interrupted
/// from work that failed.
pub fn interrupted_by_connection(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut next = Some(error);
    while let Some(error) = next {
        if error
            .downcast_ref::<TaskError>()
            .is_some_and(TaskError::is_connection)
            || error
                .downcast_ref::<veoveo_platform_store::StoreError>()
                .is_some_and(veoveo_platform_store::StoreError::is_connection)
            || error
                .downcast_ref::<sqlx::Error>()
                .is_some_and(veoveo_platform_store::is_connection_error)
        {
            return true;
        }
        next = error.source();
    }
    false
}
