//! Durable Task state as read from the database.
use std::{collections::BTreeSet, fmt};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use veoveo_types::{ResourceUri, ServerSlug, TaskId, TaskTypeName};

use crate::{InstanceId, RecoveryClass, TaskOwner, TaskStatus};

/// A consumer's claim that keeps a terminal Task from retention pruning until the
/// consumer acknowledges its result: 1 to 256 bytes without a control character.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct TaskRetentionPin(String);

impl TaskRetentionPin {
    pub fn new(value: impl Into<String>) -> Result<Self, TaskRetentionPinError> {
        let value = value.into();
        if !is_valid_key(&value) {
            return Err(TaskRetentionPinError);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TaskRetentionPin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TaskRetentionPin {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("task retention pin is empty, too long, or contains a control character")]
pub struct TaskRetentionPinError;

/// The admission rule of `tasks.is_valid_key`: 1 to 256 UTF-8 bytes without a control
/// character.
pub(crate) fn is_valid_key(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

/// One durable Task. The owner instance names the process that currently executes it.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskSnapshot {
    pub task_id: TaskId,
    pub owner: TaskOwner,
    pub server: ServerSlug,
    pub task_type: TaskTypeName,
    pub request: Value,
    pub recovery_class: RecoveryClass,
    pub status: TaskStatus,
    pub status_message: Option<String>,
    pub progress: f64,
    /// `Some(Value::Null)` is a present JSON null result.
    pub result: Option<Value>,
    pub result_uri: Option<ResourceUri>,
    pub error: Option<TaskFailure>,
    pub idempotency_key: Option<String>,
    pub owner_instance: Option<InstanceId>,
    /// Advances on every committed change; worker transitions compare it.
    pub revision: i64,
    pub cancel_requested_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub retention_expires_at: Option<DateTime<Utc>>,
    pub retention_pins: BTreeSet<TaskRetentionPin>,
    pub ttl_ms: Option<u64>,
    pub poll_interval_ms: Option<u64>,
}

impl TaskSnapshot {
    pub fn is_terminal(&self) -> bool {
        crate::transition::is_terminal(self.status)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskFailure {
    pub code: String,
    pub message: String,
    /// `Some(Value::Null)` is present JSON null details.
    #[serde(
        default,
        deserialize_with = "present_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub details: Option<Value>,
}

fn present_value<'de, D: serde::Deserializer<'de>>(decoder: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(decoder).map(Some)
}

impl TaskFailure {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    /// The failure recorded when an `interrupted_indeterminate` Task loses its owner.
    pub fn interrupted_indeterminate() -> Self {
        Self::new(
            "interrupted_indeterminate",
            "This task was interrupted and may have partly completed. It won't be retried automatically. Check the result before running it again.",
        )
    }
}

/// The order of a page of a caller's Tasks, by creation time and then ID.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, veoveo_types::Vocabulary)]
pub enum TaskPageOrder {
    /// The oldest Task first.
    OldestFirst,
    /// The newest Task first.
    NewestFirst,
}

/// A seek position and ordering for a caller-owned Task query.
///
/// A position grants no access: every query applies its current owner, type and label
/// selection, and a page in another order refuses the position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskPageCursor {
    order: TaskPageOrder,
    created_at: DateTime<Utc>,
    task_id: TaskId,
}

impl TaskPageCursor {
    /// Construct a seek position, without admitting it to any owner or collection.
    ///
    /// `OwnerTaskQuery::page` still applies the current owner, task types and labels
    /// and verifies that the requested order matches this position.
    pub fn from_position(order: TaskPageOrder, created_at: DateTime<Utc>, task_id: TaskId) -> Self {
        Self {
            order,
            created_at,
            task_id,
        }
    }

    pub(crate) fn after(order: TaskPageOrder, last: &TaskSnapshot) -> Self {
        Self::from_position(order, last.created_at, last.task_id)
    }

    /// The order of the page that issued the cursor; the next page reads in it.
    pub fn order(&self) -> TaskPageOrder {
        self.order
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub fn task_id(&self) -> TaskId {
        self.task_id
    }
}

#[derive(Clone, Debug)]
pub struct TaskPage {
    pub items: Vec<TaskSnapshot>,
    pub next_cursor: Option<TaskPageCursor>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TaskPayloadState {
    Completed(Value),
    Failed(TaskFailure),
    Cancelled,
    Running,
    Unknown,
}

impl From<Option<TaskSnapshot>> for TaskPayloadState {
    fn from(snapshot: Option<TaskSnapshot>) -> Self {
        let Some(snapshot) = snapshot else {
            return Self::Unknown;
        };
        match snapshot.status {
            TaskStatus::Succeeded => snapshot.result.map_or_else(
                || {
                    Self::Failed(TaskFailure::new(
                        "missing_result",
                        "completed task has no durable result",
                    ))
                },
                Self::Completed,
            ),
            TaskStatus::Failed => Self::Failed(
                snapshot
                    .error
                    .unwrap_or_else(|| TaskFailure::new("task_failed", "task failed")),
            ),
            TaskStatus::Cancelled => Self::Cancelled,
            _ => Self::Running,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_follow_the_database_admission_rule() {
        for valid in ["a", "agent:episode:1", &"é".repeat(128)] {
            assert!(TaskRetentionPin::new(valid).is_ok(), "{valid}");
        }
        for invalid in ["", "\n", "a\u{0085}b", &"é".repeat(129)] {
            assert!(TaskRetentionPin::new(invalid).is_err(), "{invalid:?}");
            assert!(
                serde_json::from_value::<TaskRetentionPin>(serde_json::json!(invalid)).is_err()
            );
        }
    }

    #[test]
    fn failure_details_keep_absent_and_null_apart() {
        for value in [
            serde_json::json!({"code": "owner.extension", "message": "failed"}),
            serde_json::json!({"code": "owner.extension", "message": "failed", "details": null}),
            serde_json::json!({"code": "x", "message": "y", "details": {"nested": [1, null]}}),
        ] {
            let failure: TaskFailure = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(failure).unwrap(), value);
        }
        assert!(serde_json::from_value::<TaskFailure>(serde_json::json!({"code": "x"})).is_err());
    }
}
