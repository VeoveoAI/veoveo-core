//! Durable steps of a Task's work. The owning worker records each step's operation
//! before the step runs and its result once it finishes, so a Task that runs again after
//! recovery returns finished steps from the database instead of repeating them.
use std::future::Future;

use chrono::{DateTime, Utc};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use uuid::Uuid;
use veoveo_types::{IdFailure, IdMetadata, IdProfile, IdProfileSpec, IdentifierError, TaskId};

use crate::{
    InstanceId, TaskError, TaskRuntime, runtime::validate_task_id, snapshot::is_valid_key,
};

/// The longest step key, in bytes.
pub const MAX_STEP_KEY_BYTES: usize = 128;

/// The name a Task's work gives one of its steps: 1 to 128 bytes without control
/// characters. The work names each step the same way on every run, so a run after
/// recovery finds the steps an earlier run recorded. A step repeated in a loop carries
/// its position in its key, such as `upload-part-3`.
#[veoveo_types::id(text(TaskStepKeys))]
pub struct StepKey(String);

fn validate_step_key(value: &str, _: IdMetadata) -> Result<(), IdentifierError> {
    if value.len() <= MAX_STEP_KEY_BYTES && is_valid_key(value) {
        Ok(())
    } else {
        Err(IdentifierError::new(
            value,
            "must be 1 to 128 bytes without control characters",
        ))
    }
}

#[doc(hidden)]
pub struct TaskStepKeys;
impl IdProfile for TaskStepKeys {
    type Error = IdentifierError;
    const PROFILE: IdProfileSpec<Self::Error> = IdProfileSpec::text(validate_step_key);
}

/// The identity of one step's operation: an RFC 9562 UUIDv7 generated when the step is
/// first dispatched and kept by every later attempt, so an external call can pass it as
/// its idempotency key.
#[veoveo_types::id(uuid(StepOperationIds), fresh, postgres)]
pub struct StepOperationId(Uuid);

#[doc(hidden)]
pub struct StepOperationIds;
impl IdProfile for StepOperationIds {
    type Error = IdentifierError;
    const PROFILE: IdProfileSpec<Self::Error> = IdProfileSpec {
        wire: veoveo_types::IdWire::InnerUuid,
        ..IdProfileSpec::generated_uuid(veoveo_types::UuidGrammar::aliases(&[7]), operation_error)
    };
}

fn operation_error(value: &str, _: IdMetadata, _: IdFailure) -> IdentifierError {
    IdentifierError::new(value, "must be an RFC 9562 UUIDv7")
}

/// What a step's closure learns about the step before it runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StepAttempt {
    /// The step's operation, the same on every attempt.
    pub operation_id: StepOperationId,
    pub dispatch: StepDispatch,
}

impl StepAttempt {
    /// An earlier attempt dispatched this step and recorded no result.
    pub fn is_unsettled(&self) -> bool {
        matches!(self.dispatch, StepDispatch::Unsettled { .. })
    }
}

/// Whether an earlier attempt dispatched the step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepDispatch {
    /// This attempt dispatched the step first.
    First,
    /// An earlier attempt dispatched the step at `dispatched_at` and recorded no result,
    /// because it failed or its process ended. Its effect may or may not have happened,
    /// so the step reconciles with the system it called before it acts again.
    Unsettled { dispatched_at: DateTime<Utc> },
}

pub(crate) struct StepRow {
    pub operation_id: StepOperationId,
    pub dispatched_at: DateTime<Utc>,
    pub result: Option<Value>,
    pub completed_at: Option<DateTime<Utc>>,
    pub first: bool,
}

impl TaskRuntime {
    /// Run one step of a Task this instance owns, or return the result an earlier
    /// attempt recorded for `key`.
    ///
    /// The runtime records the step's operation before `run` starts. `run` receives a
    /// [`StepAttempt`] whose dispatch is [`StepDispatch::Unsettled`] when an earlier
    /// attempt dispatched the step without recording a result. A successful result is
    /// stored as JSON and returned as it decodes from that JSON, which every later call
    /// for `key` returns without running `run`. An error from `run` records nothing and
    /// returns as it is. The Task must be running, waiting or cancel-requested, and
    /// every write requires this instance as its owner.
    pub async fn step<T, E, F, Fut>(&self, task_id: TaskId, key: &StepKey, run: F) -> Result<T, E>
    where
        T: Serialize + DeserializeOwned,
        E: From<TaskError>,
        F: FnOnce(StepAttempt) -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let task_id = validate_task_id(task_id)?;
        let step = self.dispatch_step(task_id, key).await?;
        if step.completed_at.is_some() {
            let result = step
                .result
                .ok_or_else(|| TaskError::invalid_record("a finished Task step has no result"))?;
            return Ok(serde_json::from_value(result).map_err(TaskError::from)?);
        }
        let attempt = StepAttempt {
            operation_id: step.operation_id,
            dispatch: if step.first {
                StepDispatch::First
            } else {
                StepDispatch::Unsettled {
                    dispatched_at: step.dispatched_at,
                }
            },
        };
        let value = run(attempt).await?;
        // A result that does not decode as it was written would differ on the next run,
        // so it is refused before it is stored.
        let stored = serde_json::to_value(&value).map_err(TaskError::from)?;
        let decoded = serde_json::from_value(stored.clone()).map_err(TaskError::from)?;
        self.complete_step(task_id, key, step.operation_id, stored)
            .await?;
        Ok(decoded)
    }

    async fn dispatch_step(&self, task_id: TaskId, key: &StepKey) -> Result<StepRow, TaskError> {
        let row = sqlx::query_file_as!(
            StepRow,
            "queries/step_dispatch.sql",
            task_id.as_uuid(),
            self.server().as_str(),
            self.instance_id() as InstanceId,
            key.as_str(),
            StepOperationId::new() as StepOperationId,
        )
        .fetch_optional(self.pool())
        .await?;
        match row {
            Some(row) => Ok(row),
            None => Err(self.rejected(task_id).await),
        }
    }

    async fn complete_step(
        &self,
        task_id: TaskId,
        key: &StepKey,
        operation_id: StepOperationId,
        result: Value,
    ) -> Result<(), TaskError> {
        let completed = sqlx::query_file_scalar!(
            "queries/step_complete.sql",
            task_id.as_uuid(),
            self.server().as_str(),
            self.instance_id() as InstanceId,
            key.as_str(),
            operation_id as StepOperationId,
            result,
        )
        .fetch_optional(self.pool())
        .await?;
        match completed {
            Some(_) => Ok(()),
            None => Err(self.rejected(task_id).await),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_keys_are_short_text_without_control_characters() {
        assert_eq!(
            StepKey::parse("upload-part-3").unwrap().as_str(),
            "upload-part-3"
        );
        assert!(StepKey::parse("k".repeat(MAX_STEP_KEY_BYTES)).is_ok());
        assert!(StepKey::parse("k".repeat(MAX_STEP_KEY_BYTES + 1)).is_err());
        // 64 two-byte characters fill the limit in bytes.
        assert!(StepKey::parse("é".repeat(64)).is_ok());
        assert!(StepKey::parse("é".repeat(65)).is_err());
        assert!(StepKey::parse("").is_err());
        assert!(StepKey::parse("line\nbreak").is_err());
        assert!(StepKey::parse("delete\u{7f}").is_err());
        assert!(serde_json::from_value::<StepKey>(serde_json::json!("tab\tkey")).is_err());
    }

    #[test]
    fn operation_ids_are_uuid_v7() {
        let fresh = StepOperationId::new();
        assert_eq!(fresh.as_uuid().get_version_num(), 7);
        assert!(StepOperationId::parse(Uuid::now_v7().to_string()).is_ok());
        assert!(StepOperationId::parse(Uuid::new_v4().to_string()).is_err());
        assert!(StepOperationId::try_from(Uuid::new_v4()).is_err());
        assert_eq!(
            serde_json::to_value(fresh).unwrap(),
            serde_json::json!(fresh.as_uuid().to_string())
        );
    }
}
