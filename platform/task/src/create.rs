//! Task creation and idempotent replay.
use std::collections::BTreeSet;

use chrono::{Datelike, TimeDelta, Utc};
use serde_json::Value;
use veoveo_types::{ServerSlug, TaskId, TaskTypeName};

use crate::{
    TaskError, TaskOwner, TaskRetentionPin, TaskRuntime, TaskSnapshot,
    row::{TaskRow, columns::*},
    snapshot::is_valid_key,
};

/// Retention of a Task created without a TTL: seven days.
const DEFAULT_RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1000;

/// A Task to create in the queued state.
/// ```compile_fail
/// use veoveo_task::CreateTask;
/// fn unchecked(mut draft: CreateTask) {
///     draft.task_type = "unchecked-operation".to_owned();
/// }
/// ```
#[derive(Clone, Debug)]
pub struct CreateTask {
    pub task_id: TaskId,
    pub owner: TaskOwner,
    pub server: ServerSlug,
    pub task_type: TaskTypeName,
    pub request: Value,
    pub recovery_class: RecoveryClass,
    /// Scoped to the owner's tenant, principal and surface and to the server.
    pub idempotency_key: Option<String>,
    pub ttl_ms: Option<u64>,
    pub poll_interval_ms: Option<u64>,
    pub retention_pins: BTreeSet<TaskRetentionPin>,
}

#[derive(Clone, Debug)]
pub struct CreateTaskResult {
    pub snapshot: TaskSnapshot,
    /// False when the idempotency key returned an existing Task.
    pub created: bool,
}

fn signed(field: &str, value: Option<u64>) -> Result<Option<i64>, TaskError> {
    value
        .map(|value| {
            i64::try_from(value).map_err(|_| {
                TaskError::invalid_record(format!("Task {field} exceeds the supported range"))
            })
        })
        .transpose()
}

impl TaskRuntime {
    /// Create a queued Task. A repeated idempotency key returns the existing Task.
    pub async fn create(&self, draft: CreateTask) -> Result<CreateTaskResult, TaskError> {
        crate::runtime::validate_task_id(draft.task_id)?;
        if draft.server != *self.server() {
            return Err(TaskError::WrongServer(draft.server.to_string()));
        }
        draft.owner.check_authority()?;
        if draft
            .idempotency_key
            .as_deref()
            .is_some_and(|key| !is_valid_key(key))
        {
            return Err(TaskError::invalid_record(
                "Task idempotency key is empty, too long, or contains a control character",
            ));
        }
        // Deadlines stay within RFC 3339's four-digit years.
        let retention_ms = i64::try_from(draft.ttl_ms.unwrap_or(DEFAULT_RETENTION_MS))
            .ok()
            .filter(|retention| {
                TimeDelta::try_milliseconds(*retention)
                    .and_then(|retention| Utc::now().checked_add_signed(retention))
                    .is_some_and(|deadline| deadline.year() <= 9999)
            })
            .ok_or_else(|| {
                TaskError::invalid_record("Task TTL exceeds supported deadline range")
            })?;
        let owner = &draft.owner;
        let tenant = owner.tenant();
        // The Task references its owner's identity records.
        let identity = self
            .platform_store()
            .ensure_identity(
                tenant.as_str(),
                owner.principal_key.as_str(),
                owner.issuer.as_str(),
                owner.subject.as_str(),
                owner.principal_kind,
            )
            .await?;
        let row = sqlx::query_file_as!(
            TaskRow,
            "queries/create.sql",
            draft.task_id.as_uuid(),
            self.server().as_str(),
            tenant.as_str(),
            identity.tenant_id as veoveo_store::TenantId,
            owner.tenant_key.as_ref().map(|tenant| tenant.as_str()),
            owner.principal_key.as_str(),
            identity.principal_id as veoveo_store::PrincipalId,
            owner.principal_kind as PrincipalKind,
            owner.issuer.as_str(),
            owner.subject.as_str(),
            owner.surface.as_str(),
            &owner
                .data_labels
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            owner.authority.work_context.as_str(),
            Json(&owner.authority) as _,
            draft.task_type.as_str(),
            draft.recovery_class as RecoveryClass,
            &draft.request,
            draft.idempotency_key.as_deref(),
            signed("TTL", draft.ttl_ms)?,
            signed("poll interval", draft.poll_interval_ms)?,
            &draft
                .retention_pins
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            retention_ms,
        )
        .fetch_optional(self.pool())
        .await?;
        if let Some(row) = row {
            return Ok(CreateTaskResult {
                snapshot: TaskSnapshot::try_from(row)?,
                created: true,
            });
        }
        // Only the idempotency index absorbs an insert.
        let Some(key) = draft.idempotency_key.as_deref() else {
            return Err(TaskError::Conflict(draft.task_id));
        };
        let existing = sqlx::query_file_as!(
            TaskRow,
            "queries/idempotent.sql",
            self.server().as_str(),
            tenant.as_str(),
            owner.principal_key.as_str(),
            owner.surface.as_str(),
            key,
        )
        .fetch_optional(self.pool())
        .await?
        .ok_or(TaskError::Conflict(draft.task_id))?;
        Ok(CreateTaskResult {
            snapshot: TaskSnapshot::try_from(existing)?,
            created: false,
        })
    }
}
