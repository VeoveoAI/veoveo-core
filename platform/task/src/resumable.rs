//! Settlement of `resume` Tasks by their owning worker.
use tokio_util::sync::CancellationToken;
use veoveo_types::TaskId;

use crate::{
    RecoveryClass, TaskError, TaskRuntime, TaskSnapshot, TaskStatus, TaskTransition,
    transition::Admission,
};

/// What a cancellation request pending when a worker settles its Task keeps of the
/// worker's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, veoveo_types::Vocabulary)]
pub enum ResumeCancellationPolicy {
    /// The Task ends `Cancelled`, whatever the worker settles.
    CancellationWins,
    /// A genuine failure stands; any other outcome becomes `Cancelled`.
    PreserveFailure,
    /// A finished outcome stands, success or genuine failure, because the cancellation
    /// arrived after the work was done; only unfinished work becomes `Cancelled`.
    PreserveOutcome,
}

impl ResumeCancellationPolicy {
    /// Whether `transition` stands against a pending cancellation request.
    fn keeps(self, transition: &TaskTransition) -> bool {
        match self {
            Self::CancellationWins => false,
            Self::PreserveFailure => matches!(transition, TaskTransition::Failed(_)),
            Self::PreserveOutcome => matches!(
                transition,
                TaskTransition::Succeeded { .. } | TaskTransition::Failed(_)
            ),
        }
    }
}

impl TaskRuntime {
    /// Settle a `resume` Task this instance owns from its current revision.
    ///
    /// A pending cancellation request turns the outcome into `Cancelled`, except what
    /// `policy` keeps: `PreserveFailure` a genuine failure, `PreserveOutcome` a success
    /// or a genuine failure. With `stop` cancelled, a success is not published: the call
    /// returns the unchanged Task, unless cancellation is pending, which it settles. A
    /// settled Task is returned as it is.
    pub async fn transition_resumable(
        &self,
        id: TaskId,
        transition: TaskTransition,
        policy: ResumeCancellationPolicy,
        stop: Option<&CancellationToken>,
    ) -> Result<TaskSnapshot, TaskError> {
        let current = self.resume_snapshot(id).await?;
        self.resume_settle(&current, &current, transition, policy, stop)
            .await
    }

    /// As `transition_resumable`, applied only while `current` is the Task's latest
    /// revision. A newer revision fails with `Conflict` unless it is a cancellation
    /// request, which settles the Task.
    pub async fn transition_resumable_if_current(
        &self,
        current: &TaskSnapshot,
        transition: TaskTransition,
        policy: ResumeCancellationPolicy,
        stop: Option<&CancellationToken>,
    ) -> Result<TaskSnapshot, TaskError> {
        let observed = self.resume_snapshot(current.task_id).await?;
        self.resume_settle(current, &observed, transition, policy, stop)
            .await
    }

    async fn resume_snapshot(&self, id: TaskId) -> Result<TaskSnapshot, TaskError> {
        self.get(id).await?.ok_or(TaskError::NotFound(id))
    }

    async fn resume_settle(
        &self,
        selected: &TaskSnapshot,
        admitted: &TaskSnapshot,
        transition: TaskTransition,
        policy: ResumeCancellationPolicy,
        stop: Option<&CancellationToken>,
    ) -> Result<TaskSnapshot, TaskError> {
        self.resume_identity(selected, admitted)?;
        if admitted.is_terminal() {
            return Ok(admitted.clone());
        }
        self.ensure_owner(admitted)?;
        match self
            .resume_dispatch(selected, admitted, transition.clone(), policy, stop)
            .await
        {
            Ok(settled) => Ok(settled),
            Err(error @ (TaskError::Conflict(_) | TaskError::InvalidTransition { .. })) => {
                let observed = self.resume_snapshot(selected.task_id).await?;
                self.resume_identity(selected, &observed)?;
                if observed.is_terminal() {
                    return Ok(observed);
                }
                self.ensure_owner(&observed)?;
                if observed.status != TaskStatus::CancelRequested {
                    return Err(error);
                }
                self.resume_dispatch(&observed, &observed, transition, policy, stop)
                    .await
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) async fn resume_dispatch(
        &self,
        current: &TaskSnapshot,
        admitted: &TaskSnapshot,
        transition: TaskTransition,
        policy: ResumeCancellationPolicy,
        stop: Option<&CancellationToken>,
    ) -> Result<TaskSnapshot, TaskError> {
        self.ensure_owner(current)?;
        let transition =
            if current.status == TaskStatus::CancelRequested && !policy.keeps(&transition) {
                TaskTransition::Cancelled
            } else {
                transition
            };
        if matches!(transition, TaskTransition::Succeeded { .. })
            && stop.is_some_and(CancellationToken::is_cancelled)
        {
            // The admitted read is newer than the caller's selection, so a stop that
            // arrived after admission still settles a pending cancellation.
            self.ensure_owner(admitted)?;
            if admitted.status == TaskStatus::CancelRequested {
                return self
                    .transition_if_current(admitted, TaskTransition::Cancelled)
                    .await;
            }
            return Ok(admitted.clone());
        }
        let admission = match policy {
            ResumeCancellationPolicy::PreserveOutcome => Admission::FinishedOverCancellation,
            _ => Admission::Worker,
        };
        self.transition_admitted(current, transition, admission)
            .await
    }

    fn resume_identity(
        &self,
        selected: &TaskSnapshot,
        current: &TaskSnapshot,
    ) -> Result<(), TaskError> {
        if current.server != *self.server() {
            return Err(TaskError::WrongServer(current.server.to_string()));
        }
        if selected.recovery_class != RecoveryClass::Resume
            || current.recovery_class != RecoveryClass::Resume
        {
            return Err(TaskError::invalid_record(
                "Resume settlement requires the resume recovery class",
            ));
        }
        if selected.task_id != current.task_id
            || selected.owner != current.owner
            || selected.request != current.request
            || selected.task_type != current.task_type
            || selected.created_at != current.created_at
            || selected.ttl_ms != current.ttl_ms
            || selected.poll_interval_ms != current.poll_interval_ms
        {
            return Err(TaskError::Conflict(selected.task_id));
        }
        Ok(())
    }
}
