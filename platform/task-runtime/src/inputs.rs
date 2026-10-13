//! Inputs a Task requests from its caller and the caller's answers.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use veoveo_types::TaskId;

use crate::{
    InstanceId, TaskError, TaskRuntime, TaskStatus, row::InputRow, snapshot::is_valid_key,
    transition::is_terminal,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskInputRequest {
    pub method: String,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskInputExchange {
    pub key: String,
    pub request: TaskInputRequest,
    pub response: Option<BTreeMap<String, Value>>,
    pub created_at: DateTime<Utc>,
    pub responded_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TaskInputSubmission {
    pub accepted: usize,
    pub ignored: usize,
}

pub(crate) type InputResponses = BTreeMap<String, BTreeMap<String, Value>>;

/// The most input requests one Task holds, which `tasks.task_input_limit` enforces.
pub const MAX_TASK_INPUTS: usize = 64;
/// The constraint name the limit's trigger reports.
const INPUT_LIMIT_CONSTRAINT: &str = "task_input_limit";

pub(crate) fn validate_input_key(key: &str) -> Result<(), TaskError> {
    if !is_valid_key(key) {
        return Err(TaskError::InvalidInputKey);
    }
    Ok(())
}

/// A Task that is cancelling or finished accepts no answers.
pub(crate) fn accepts_input(status: TaskStatus) -> Result<(), TaskError> {
    if is_terminal(status) || status == TaskStatus::CancelRequested {
        return Err(TaskError::InvalidTransition {
            from: status,
            to: TaskStatus::Running,
        });
    }
    Ok(())
}

pub(crate) fn object(map: BTreeMap<String, Value>) -> Value {
    Value::Object(map.into_iter().collect())
}

impl TaskRuntime {
    /// Ask the caller for input. The Task waits until a caller answers; this instance
    /// must own it. A Task holds at most [`MAX_TASK_INPUTS`] requests, answered or not.
    pub async fn request_input(
        &self,
        task_id: TaskId,
        key: &str,
        request: TaskInputRequest,
    ) -> Result<TaskInputExchange, TaskError> {
        validate_input_key(key)?;
        if !is_valid_key(&request.method) {
            return Err(TaskError::invalid_record(
                "task input method is empty, too long, or contains a control character",
            ));
        }
        let row = sqlx::query_file_as!(
            InputRow,
            "queries/request_input.sql",
            task_id.as_uuid(),
            self.server().as_str(),
            self.instance_id() as InstanceId,
            key,
            request.method,
            object(request.params),
        )
        .fetch_optional(self.pool())
        .await
        .map_err(|error| match &error {
            sqlx::Error::Database(database) if database.is_unique_violation() => {
                TaskError::DuplicateInputKey(key.to_owned())
            }
            sqlx::Error::Database(database)
                if database.constraint() == Some(INPUT_LIMIT_CONSTRAINT) =>
            {
                TaskError::InputLimit(task_id)
            }
            _ => TaskError::Database(error),
        })?;
        match row {
            Some(row) => TaskInputExchange::try_from(row),
            None => match self.get(task_id).await? {
                Some(current) if current.owner_instance == Some(self.instance_id()) => {
                    Err(TaskError::InvalidTransition {
                        from: current.status,
                        to: TaskStatus::Waiting,
                    })
                }
                Some(_) => Err(TaskError::NotOwner(task_id)),
                None => Err(TaskError::NotFound(task_id)),
            },
        }
    }

    /// One input exchange of this server's Task.
    pub async fn input_exchange(
        &self,
        task_id: TaskId,
        key: &str,
    ) -> Result<Option<TaskInputExchange>, TaskError> {
        sqlx::query_file_as!(
            InputRow,
            "queries/input_exchange.sql",
            task_id.as_uuid(),
            self.server().as_str(),
            key,
        )
        .fetch_optional(self.pool())
        .await?
        .map(TaskInputExchange::try_from)
        .transpose()
    }

    /// Unanswered inputs of this server's Task, keyed by request key.
    pub async fn outstanding_inputs(
        &self,
        task_id: TaskId,
    ) -> Result<BTreeMap<String, TaskInputRequest>, TaskError> {
        sqlx::query_file_as!(
            InputRow,
            "queries/outstanding_inputs.sql",
            task_id.as_uuid(),
            self.server().as_str(),
        )
        .fetch_all(self.pool())
        .await?
        .into_iter()
        .map(|row| TaskInputExchange::try_from(row).map(|input| (input.key, input.request)))
        .collect()
    }

    /// Answer inputs on the worker's behalf without a caller selection. Callers answer
    /// through `OwnerTaskQuery::submit_input_responses`.
    pub async fn submit_input_responses(
        &self,
        task_id: TaskId,
        responses: InputResponses,
    ) -> Result<TaskInputSubmission, TaskError> {
        let current = self
            .get(task_id)
            .await?
            .ok_or(TaskError::NotFound(task_id))?;
        accepts_input(current.status)?;
        let mut submission = TaskInputSubmission::default();
        for (key, response) in responses {
            validate_input_key(&key)?;
            let answer = sqlx::query_file!(
                "queries/answer_input.sql",
                task_id.as_uuid(),
                self.server().as_str(),
                key,
                object(response),
            )
            .fetch_one(self.pool())
            .await?;
            if !answer.admitted {
                return Err(self.input_rejected(task_id).await);
            }
            if answer.accepted {
                submission.accepted += 1;
            } else {
                submission.ignored += 1;
            }
        }
        Ok(submission)
    }

    /// Classify an answer whose parent Task stopped accepting input.
    pub(crate) async fn input_rejected(&self, task_id: TaskId) -> TaskError {
        match self.get(task_id).await {
            Ok(Some(current)) => accepts_input(current.status)
                .err()
                .unwrap_or(TaskError::Conflict(task_id)),
            Ok(None) => TaskError::NotFound(task_id),
            Err(error) => error,
        }
    }
}
