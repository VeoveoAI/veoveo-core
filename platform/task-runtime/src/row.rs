//! Rows of `tasks.task` and `tasks.task_input` as the typed queries return them.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;
use veoveo_types::{ResourceUri, TaskId};

use crate::{
    TaskError, TaskFailure, TaskInputExchange, TaskInputRequest, TaskOwner, TaskRetentionPin,
    TaskSnapshot,
};

/// Types the Task queries decode columns into, named by their `AS "column: Type"`
/// overrides. Modules that run those queries import them from here.
pub(crate) mod columns {
    pub(crate) use crate::{InstanceId, PrincipalKind, RecoveryClass, TaskStatus};
    pub(crate) use sqlx::types::Json;
    pub(crate) use veoveo_types::InvocationAuthority;
}
use columns::*;

pub(crate) struct TaskRow {
    pub id: Uuid,
    pub server: String,
    pub tenant: String,
    pub owner_tenant: Option<String>,
    pub owner_principal: String,
    pub owner_kind: PrincipalKind,
    pub owner_issuer: String,
    pub owner_subject: String,
    pub profile: String,
    pub data_labels: Vec<String>,
    pub work_context: String,
    pub authority: Json<InvocationAuthority>,
    pub task_type: String,
    pub recovery_class: RecoveryClass,
    pub input: Value,
    pub idempotency_key: Option<String>,
    pub ttl_ms: Option<i64>,
    pub poll_interval_ms: Option<i64>,
    pub status: TaskStatus,
    pub status_message: Option<String>,
    pub progress: f64,
    pub result: Option<Value>,
    pub result_uri: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub error_details: Option<Value>,
    pub owner_instance: Option<InstanceId>,
    pub revision: i64,
    pub retention_pins: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub cancel_requested_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub retention_expires_at: Option<DateTime<Utc>>,
}

fn admit<T: std::str::FromStr>(field: &str, value: &str) -> Result<T, TaskError>
where
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|error| TaskError::invalid_record(format!("stored Task {field}: {error}")))
}

fn unsigned(field: &str, value: Option<i64>) -> Result<Option<u64>, TaskError> {
    value
        .map(|value| {
            u64::try_from(value)
                .map_err(|_| TaskError::invalid_record(format!("stored Task {field} is negative")))
        })
        .transpose()
}

impl TryFrom<TaskRow> for TaskSnapshot {
    type Error = TaskError;

    fn try_from(row: TaskRow) -> Result<Self, Self::Error> {
        let owner = TaskOwner {
            principal_key: admit("owner principal", &row.owner_principal)?,
            principal_kind: row.owner_kind,
            issuer: admit("owner issuer", &row.owner_issuer)?,
            subject: admit("owner subject", &row.owner_subject)?,
            profile: admit("profile", &row.profile)?,
            tenant_key: row
                .owner_tenant
                .as_deref()
                .map(|tenant| admit("owner tenant", tenant))
                .transpose()?,
            data_labels: row
                .data_labels
                .iter()
                .map(|label| admit("data label", label))
                .collect::<Result<_, _>>()?,
            authority: row.authority.0,
        };
        if owner.tenant().as_str() != row.tenant
            || owner.authority.work_context.as_str() != row.work_context
        {
            return Err(TaskError::invalid_record(
                "stored Task owner disagrees with its tenant or Work Context",
            ));
        }
        let error = match (row.error_code, row.error_message) {
            (Some(code), Some(message)) => Some(TaskFailure {
                code,
                message,
                details: row.error_details,
            }),
            (None, None) if row.error_details.is_none() => None,
            _ => {
                return Err(TaskError::invalid_record(
                    "stored Task failure is incomplete",
                ));
            }
        };
        Ok(Self {
            task_id: TaskId::from_uuid(row.id),
            owner,
            server: admit("server", &row.server)?,
            task_type: admit("operation", &row.task_type)?,
            request: row.input,
            recovery_class: row.recovery_class,
            status: row.status,
            status_message: row.status_message,
            progress: row.progress,
            result: row.result,
            result_uri: row
                .result_uri
                .map(ResourceUri::new)
                .transpose()
                .map_err(|error| {
                    TaskError::invalid_record(format!("stored Task product address: {error}"))
                })?,
            error,
            idempotency_key: row.idempotency_key,
            owner_instance: row.owner_instance,
            revision: row.revision,
            cancel_requested_at: row.cancel_requested_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
            started_at: row.started_at,
            completed_at: row.completed_at,
            retention_expires_at: row.retention_expires_at,
            retention_pins: row
                .retention_pins
                .into_iter()
                .map(TaskRetentionPin::new)
                .collect::<Result<_, _>>()
                .map_err(|error| TaskError::invalid_record(error.to_string()))?,
            ttl_ms: unsigned("TTL", row.ttl_ms)?,
            poll_interval_ms: unsigned("poll interval", row.poll_interval_ms)?,
        })
    }
}

pub(crate) struct InputRow {
    pub request_key: String,
    pub method: String,
    pub params: Value,
    pub response: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub responded_at: Option<DateTime<Utc>>,
}

fn object(field: &str, value: Value) -> Result<BTreeMap<String, Value>, TaskError> {
    match value {
        Value::Object(object) => Ok(object.into_iter().collect()),
        _ => Err(TaskError::invalid_record(format!(
            "stored Task input {field} is not an object"
        ))),
    }
}

impl TryFrom<InputRow> for TaskInputExchange {
    type Error = TaskError;

    fn try_from(row: InputRow) -> Result<Self, Self::Error> {
        Ok(Self {
            key: row.request_key,
            request: TaskInputRequest {
                method: row.method,
                params: object("parameters", row.params)?,
            },
            response: row
                .response
                .map(|response| object("response", response))
                .transpose()?,
            created_at: row.created_at,
            responded_at: row.responded_at,
        })
    }
}
