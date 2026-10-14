//! Durable Tasks in the installation's PostgreSQL database.
//!
//! Every Task is a row of `tasks.task`. A process instance owns the Tasks it claims,
//! worker writes compare that ownership, and the database moves a dead instance's Tasks
//! to their recovery state. A worker records each step of a Task's work, so a Task that
//! runs again after recovery skips the steps it finished. Callers reach their own Tasks through
//! `TaskRuntime::for_owner`, and each server's notification channel, `tasks.<server>`,
//! wakes that server's workers and subscribers. A Task that hands work to a provider
//! records it as an operation and parks on it, holding no worker until a provider event,
//! a due status check or the operation's deadline queues it again.

mod claims;
mod config;
mod create;
mod error;
mod inputs;
mod operations;
mod owner;
mod owner_query;
mod resumable;
mod retention;
mod row;
mod runtime;
mod signals;
mod snapshot;
mod steps;
mod subscriptions;
#[cfg(test)]
#[allow(dead_code, reason = "the unit tests use part of the shared fixture")]
#[path = "../../../testing/fixtures/store.rs"]
mod test_store;
#[cfg(test)]
mod test_support;
mod transition;
mod work;

pub use config::TaskRuntimeConfig;
pub use create::{CreateTask, CreateTaskResult};
pub use error::{TaskError, interrupted_by_connection};
pub use inputs::{MAX_TASK_INPUTS, TaskInputExchange, TaskInputRequest, TaskInputSubmission};
pub use operations::{
    CallbackToken, CheckObservation, CheckSchedule, Completion, DUE_INTERVAL, Delivery, DuePass,
    EventTarget, ExternalJobId, MAX_OPERATION_KEY_BYTES, Operation, OperationAttempt,
    OperationCalls, OperationEvent, OperationEventId, OperationKey, OperationSpec, OperationState,
    ProviderName, Submission, WaitOutcome,
};
pub use owner::{TaskOwner, TaskSurface};
pub use owner_query::OwnerTaskQuery;
pub use resumable::ResumeCancellationPolicy;
pub use retention::RETENTION_INTERVAL;
pub use runtime::{TaskRuntime, parse_task_id};
pub use snapshot::{
    TaskFailure, TaskPage, TaskPageCursor, TaskPayloadState, TaskRetentionPin,
    TaskRetentionPinError, TaskSnapshot,
};
pub use steps::{MAX_STEP_KEY_BYTES, StepAttempt, StepDispatch, StepKey, StepOperationId};
pub use subscriptions::{OwnerTaskSubscription, TaskUpdateStream};
pub use transition::TaskTransition;
pub use veoveo_platform_store::{
    Instance, InstanceId, InstancePhase, InstanceState, MaintenanceBatch, PrincipalKind, Pruned,
};
pub use veoveo_task_contract::{RecoveryClass, TaskStatus};
pub use work::{CLAIM_RETRY_FIRST_DELAY, CLAIM_RETRY_MAX_DELAY, TaskWork};
