//! Work claims. A claim records this runtime's instance as the Task's owner and moves
//! the Task to running in one statement; `FOR UPDATE SKIP LOCKED` lets concurrent
//! workers claim different Tasks without waiting on each other.
use veoveo_types::TaskId;

use crate::{
    TaskError, TaskRuntime, TaskSnapshot,
    row::{TaskRow, columns::*},
};

impl TaskRuntime {
    /// Claim one queued Task of this server.
    pub async fn claim(&self, task_id: TaskId) -> Result<TaskSnapshot, TaskError> {
        let task_id = crate::runtime::validate_task_id(task_id)?;
        self.check_instance()?;
        let row = sqlx::query_file_as!(
            TaskRow,
            "queries/claim.sql",
            task_id.as_uuid(),
            self.server().as_str(),
            self.instance_id() as InstanceId,
        )
        .fetch_optional(self.pool())
        .await?;
        if let Some(row) = row {
            return TaskSnapshot::try_from(row);
        }
        match self.get(task_id).await? {
            None => Err(TaskError::NotFound(task_id)),
            Some(current) if current.status != TaskStatus::Queued => {
                Err(TaskError::InvalidTransition {
                    from: current.status,
                    to: TaskStatus::Running,
                })
            }
            // Another worker holds the row lock while it claims or changes the Task.
            Some(_) => Err(TaskError::Conflict(task_id)),
        }
    }

    /// Claim the oldest queued Task of this server that no other worker is claiming.
    pub async fn claim_next(&self) -> Result<Option<TaskSnapshot>, TaskError> {
        self.check_instance()?;
        sqlx::query_file_as!(
            TaskRow,
            "queries/claim_next.sql",
            self.server().as_str(),
            self.instance_id() as InstanceId,
        )
        .fetch_optional(self.pool())
        .await?
        .map(TaskSnapshot::try_from)
        .transpose()
    }
}
