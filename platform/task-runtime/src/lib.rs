//! Durable MCP Tasks in the installation's PostgreSQL database.
//!
//! Every Task is a row of `tasks.task`. A process instance owns the Tasks it claims,
//! worker writes compare that ownership, and the database moves a dead instance's Tasks
//! to their recovery state. Callers reach their own Tasks through
//! `TaskRuntime::for_owner`, and each server's notification channel, `tasks.<server>`,
//! wakes that server's workers and subscribers.

mod claims;
mod config;
mod create;
mod error;
pub mod hosting;
mod inputs;
mod mcp;
mod owner;
mod owner_query;
mod resumable;
mod row;
mod runtime;
mod service;
mod signals;
mod snapshot;
mod subscriptions;
#[cfg(test)]
#[allow(dead_code, reason = "the unit tests use part of the shared fixture")]
#[path = "../../../testing/fixtures/store.rs"]
mod test_store;
mod transition;
mod work;

pub use config::TaskRuntimeConfig;
pub use create::{CreateTask, CreateTaskResult};
pub use error::TaskError;
pub use hosting::{DurableListener, DurableTasks, TasksOnly, WithResources};
pub use inputs::{TaskInputExchange, TaskInputRequest, TaskInputSubmission};
pub use mcp::{mcp_task_completion, project_snapshot, task_seed};
pub use owner::TaskOwner;
pub use owner_query::OwnerTaskQuery;
pub use resumable::ResumeCancellationPolicy;
pub use runtime::TaskRuntime;
pub use service::{
    DurableTaskService, DurableTaskSubscription, DurableTaskUpdateStream,
    TASK_RETENTION_PIN_META_KEY, authorized_snapshot, cancel_durable_task, durable_input_responses,
    get_durable_task, listen_durable_subscriptions, restore_task_retention_meta, retention_pins,
    start_durable_tool_task, subscribe_authorized_snapshots, subscribe_durable_tasks,
    update_durable_task,
};
pub use snapshot::{
    TaskFailure, TaskPage, TaskPageCursor, TaskPayloadState, TaskRetentionPin,
    TaskRetentionPinError, TaskSnapshot,
};
pub use subscriptions::{OwnerTaskSubscription, TaskUpdateStream};
pub use transition::TaskTransition;
pub use veoveo_platform_store::{Instance, InstanceId, PrincipalKind};
pub use veoveo_task_contract::{RecoveryClass, TaskStatus};
pub use work::TaskWork;
