//! Worker transitions. The worker's instance must own the Task, and a transition
//! derived from a snapshot applies only while that snapshot is current.
use serde_json::Value;
use veoveo_types::{ResourceUri, TaskId};

use crate::{
    TaskError, TaskFailure, TaskRuntime, TaskSnapshot,
    row::{TaskRow, columns::*},
};

/// A state the owning worker moves its Task to. Callers request cancellation with
/// `TaskRuntime::cancel` or `OwnerTaskQuery::cancel`.
#[derive(Clone, Debug, PartialEq)]
pub enum TaskTransition {
    Running {
        message: String,
        progress: f64,
    },
    Waiting {
        message: String,
        progress: f64,
    },
    Succeeded {
        message: String,
        result: Value,
        result_uri: Option<ResourceUri>,
    },
    Failed(TaskFailure),
    Cancelled,
}

impl TaskTransition {
    pub(crate) fn status(&self) -> TaskStatus {
        match self {
            Self::Running { .. } => TaskStatus::Running,
            Self::Waiting { .. } => TaskStatus::Waiting,
            Self::Succeeded { .. } => TaskStatus::Succeeded,
            Self::Failed(_) => TaskStatus::Failed,
            Self::Cancelled => TaskStatus::Cancelled,
        }
    }

    fn message(&self) -> &str {
        match self {
            Self::Running { message, .. }
            | Self::Waiting { message, .. }
            | Self::Succeeded { message, .. } => message,
            Self::Failed(failure) => &failure.message,
            Self::Cancelled => "Cancelled",
        }
    }

    fn progress(&self, current: f64) -> f64 {
        match self {
            Self::Running { progress, .. } | Self::Waiting { progress, .. } => *progress,
            Self::Succeeded { .. } => 1.0,
            Self::Failed(_) | Self::Cancelled => current,
        }
    }
}

pub(crate) fn is_terminal(status: TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Succeeded | TaskStatus::Failed | TaskStatus::Cancelled
    )
}

/// The worker transitions a write admits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Admission {
    /// The transitions of [`allowed_transition`].
    Worker,
    /// Also a success over a pending cancellation request, which
    /// `ResumeCancellationPolicy::PreserveOutcome` keeps because the work finished before
    /// the request arrived.
    FinishedOverCancellation,
}

/// Worker transitions start from a claimed Task: running, waiting or cancel-requested.
pub(crate) fn allowed_transition(from: TaskStatus, to: TaskStatus) -> bool {
    match from {
        TaskStatus::Running => matches!(
            to,
            TaskStatus::Running | TaskStatus::Waiting | TaskStatus::Succeeded | TaskStatus::Failed
        ),
        TaskStatus::Waiting => matches!(
            to,
            TaskStatus::Running | TaskStatus::Waiting | TaskStatus::Succeeded | TaskStatus::Failed
        ),
        TaskStatus::CancelRequested => matches!(to, TaskStatus::Cancelled | TaskStatus::Failed),
        TaskStatus::Queued | TaskStatus::Succeeded | TaskStatus::Failed | TaskStatus::Cancelled => {
            false
        }
    }
}

impl TaskRuntime {
    /// Apply a worker transition to the current revision of a Task this instance owns.
    pub async fn transition(
        &self,
        task_id: TaskId,
        transition: TaskTransition,
    ) -> Result<TaskSnapshot, TaskError> {
        let current = self
            .get(task_id)
            .await?
            .ok_or(TaskError::NotFound(task_id))?;
        self.transition_if_current(&current, transition).await
    }

    /// Apply a worker transition only while `current` is the Task's latest revision, so
    /// work derived from that snapshot never overwrites newer state.
    pub async fn transition_if_current(
        &self,
        current: &TaskSnapshot,
        transition: TaskTransition,
    ) -> Result<TaskSnapshot, TaskError> {
        self.transition_admitted(current, transition, Admission::Worker)
            .await
    }

    /// As `transition_if_current`, admitting the transitions `admission` names.
    pub(crate) async fn transition_admitted(
        &self,
        current: &TaskSnapshot,
        transition: TaskTransition,
        admission: Admission,
    ) -> Result<TaskSnapshot, TaskError> {
        if current.server != *self.server() {
            return Err(TaskError::WrongServer(current.server.to_string()));
        }
        let next = transition.status();
        let finished_over_cancellation = admission == Admission::FinishedOverCancellation
            && current.status == TaskStatus::CancelRequested
            && next == TaskStatus::Succeeded;
        if !allowed_transition(current.status, next) && !finished_over_cancellation {
            return Err(TaskError::InvalidTransition {
                from: current.status,
                to: next,
            });
        }
        let progress = transition.progress(current.progress);
        if !progress.is_finite() || !(0.0..=1.0).contains(&progress) {
            return Err(TaskError::InvalidProgress);
        }
        let (result, result_uri, failure) = match &transition {
            TaskTransition::Succeeded {
                result, result_uri, ..
            } => (
                Some(result.clone()),
                result_uri.as_ref().map(ToString::to_string),
                None,
            ),
            TaskTransition::Failed(failure) => (None, None, Some(failure.clone())),
            _ => (None, None, None),
        };
        let (error_code, error_message, error_details) = match failure {
            Some(failure) => (Some(failure.code), Some(failure.message), failure.details),
            None => (None, None, None),
        };
        let row = sqlx::query_file_as!(
            TaskRow,
            "queries/transition.sql",
            current.task_id.as_uuid(),
            self.server().as_str(),
            self.instance_id() as InstanceId,
            current.revision,
            next as TaskStatus,
            transition.message(),
            progress,
            result,
            result_uri,
            error_code,
            error_message,
            error_details,
        )
        .fetch_optional(self.pool())
        .await?;
        match row {
            Some(row) => TaskSnapshot::try_from(row),
            None => Err(self.rejected(current.task_id).await),
        }
    }

    /// Give up this instance's claim on a Task it runs without an outcome, as the end of
    /// the instance would: a `resume` Task is queued again and continues from its
    /// finished steps, a requested cancellation completes, and any other Task ends as
    /// interrupted. A Task this instance no longer owns stays as it is, and the call
    /// fails with [`TaskError::NotOwner`].
    pub async fn release(&self, task_id: TaskId) -> Result<TaskSnapshot, TaskError> {
        let task_id = crate::runtime::validate_task_id(task_id)?;
        let row = sqlx::query_file_as!(
            TaskRow,
            "queries/release.sql",
            task_id.as_uuid(),
            self.server().as_str(),
            self.instance_id() as InstanceId,
        )
        .fetch_optional(self.pool())
        .await?;
        match row {
            Some(row) => TaskSnapshot::try_from(row),
            None => Err(self.rejected(task_id).await),
        }
    }

    /// Classify a worker write that matched no row.
    pub(crate) async fn rejected(&self, task_id: TaskId) -> TaskError {
        match self.get(task_id).await {
            Ok(None) => TaskError::NotFound(task_id),
            Ok(Some(durable)) if durable.owner_instance != Some(self.instance_id()) => {
                TaskError::NotOwner(task_id)
            }
            Ok(Some(_)) => TaskError::Conflict(task_id),
            Err(error) => error,
        }
    }

    /// Fail unless this runtime's instance owns the Task in `snapshot`.
    pub fn ensure_owner(&self, snapshot: &TaskSnapshot) -> Result<(), TaskError> {
        if snapshot.owner_instance != Some(self.instance_id()) {
            return Err(TaskError::NotOwner(snapshot.task_id));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_transitions_start_from_claimed_states() {
        assert!(allowed_transition(
            TaskStatus::Running,
            TaskStatus::Succeeded
        ));
        assert!(allowed_transition(TaskStatus::Waiting, TaskStatus::Running));
        assert!(allowed_transition(
            TaskStatus::CancelRequested,
            TaskStatus::Cancelled
        ));
        assert!(allowed_transition(
            TaskStatus::CancelRequested,
            TaskStatus::Failed
        ));
        assert!(!allowed_transition(TaskStatus::Queued, TaskStatus::Running));
        assert!(!allowed_transition(
            TaskStatus::CancelRequested,
            TaskStatus::Succeeded
        ));
        assert!(!allowed_transition(
            TaskStatus::Running,
            TaskStatus::Cancelled
        ));
        for terminal in [
            TaskStatus::Succeeded,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ] {
            assert!(is_terminal(terminal));
            for next in TaskStatus::ALL {
                assert!(!allowed_transition(terminal, *next));
            }
        }
    }
}
