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
    #[error("task input key is empty, too long, or contains a control character")]
    InvalidInputKey,
    #[error("task input key `{0}` has already been used")]
    DuplicateInputKey(String),
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
    pub(crate) fn invalid_record(message: impl Into<String>) -> Self {
        Self::InvalidRecord(message.into())
    }
}
