//! Caller-facing Task access. Every read and write selects the caller's Tasks in SQL
//! before decoding, so a missing Task and another caller's Task look the same.
use std::collections::BTreeSet;

use veoveo_types::{TaskId, TaskTypeName};

use crate::{
    TaskError, TaskInputSubmission, TaskOwner, TaskPage, TaskPageCursor, TaskRuntime, TaskSnapshot,
    inputs::{InputResponses, accepts_input, object, validate_input_key},
    owner::OwnerScope,
    row::{TaskRow, columns::*},
};

/// The most Task types one query selects.
const MAX_TASK_TYPES: usize = 32;
/// The largest page.
const MAX_PAGE: usize = 1000;

/// Tasks of one owner on this runtime's server. The default selects every operation;
/// `of_type`, `of_types` and `in_work_context` narrow the selection.
#[derive(Clone)]
pub struct OwnerTaskQuery {
    pub(crate) runtime: TaskRuntime,
    owner: TaskOwner,
    task_types: Option<BTreeSet<TaskTypeName>>,
    work_context: bool,
}

impl TaskRuntime {
    pub fn for_owner(&self, owner: &TaskOwner) -> OwnerTaskQuery {
        OwnerTaskQuery {
            runtime: self.clone(),
            owner: owner.clone(),
            task_types: None,
            work_context: false,
        }
    }
}

impl OwnerTaskQuery {
    /// Also require the Task's Work Context to be the caller's. Invocation admission
    /// grants membership and permission; this narrows the rows.
    pub fn in_work_context(mut self) -> Result<Self, TaskError> {
        self.owner.check_authority()?;
        self.work_context = true;
        Ok(self)
    }

    /// Select one operation type.
    /// ```compile_fail
    /// use veoveo_task_runtime::{TaskRuntime, TaskOwner};
    /// fn query(runtime: &TaskRuntime, owner: &TaskOwner) {
    ///     runtime.for_owner(owner).of_type("untyped-operation");
    /// }
    /// ```
    pub fn of_type(mut self, kind: TaskTypeName) -> Self {
        self.task_types = Some(BTreeSet::from([kind]));
        self
    }

    /// Select 1–32 operation types, replacing the previous selection. An empty
    /// selection is an error, never an unrestricted query.
    pub fn of_types(
        mut self,
        kinds: impl IntoIterator<Item = TaskTypeName>,
    ) -> Result<Self, TaskError> {
        let kinds = kinds
            .into_iter()
            .take(MAX_TASK_TYPES + 1)
            .collect::<Vec<_>>();
        if !(1..=MAX_TASK_TYPES).contains(&kinds.len()) {
            return Err(TaskError::InvalidPageQuery);
        }
        self.task_types = Some(kinds.into_iter().collect());
        Ok(self)
    }

    pub(crate) fn scope(&self) -> OwnerScope {
        OwnerScope {
            server: self.runtime.server().to_string(),
            tenant: self.owner.tenant().to_string(),
            principal: self.owner.principal_key.to_string(),
            surface: self.owner.surface.to_string(),
            owner_tenant: self.owner.tenant_key.as_ref().map(ToString::to_string),
            labels: self
                .owner
                .data_labels
                .iter()
                .map(ToString::to_string)
                .collect(),
            task_types: self
                .task_types
                .as_ref()
                .map(|kinds| kinds.iter().map(ToString::to_string).collect()),
            work_context: self
                .work_context
                .then(|| self.owner.authority.work_context.to_string()),
        }
    }

    /// The caller's Task, or `None` when it is missing or not the caller's.
    /// ```compile_fail
    /// use veoveo_task_runtime::{TaskOwner, TaskRuntime};
    /// async fn wrong(runtime: &TaskRuntime, owner: &TaskOwner) {
    ///     runtime.for_owner(owner).get("raw-task-id").await;
    /// }
    /// ```
    pub async fn get(&self, task: TaskId) -> Result<Option<TaskSnapshot>, TaskError> {
        let task = crate::runtime::validate_task_id(task)?;
        let scope = self.scope();
        sqlx::query_file_as!(
            TaskRow,
            "queries/owner/get.sql",
            scope.server,
            scope.tenant,
            scope.principal,
            scope.surface,
            scope.owner_tenant,
            &scope.labels,
            scope.task_types.as_deref(),
            scope.work_context,
            task.as_uuid(),
        )
        .fetch_optional(self.runtime.pool())
        .await?
        .map(TaskSnapshot::try_from)
        .transpose()
    }

    /// The caller's Tasks in creation order, at most `limit` (1–1000) after `after`.
    /// A cursor is a position; each page applies the caller's current selection.
    pub async fn page(
        &self,
        after: Option<&TaskPageCursor>,
        limit: usize,
    ) -> Result<TaskPage, TaskError> {
        if !(1..=MAX_PAGE).contains(&limit) {
            return Err(TaskError::InvalidPageQuery);
        }
        let scope = self.scope();
        let rows = sqlx::query_file_as!(
            TaskRow,
            "queries/owner/page.sql",
            scope.server,
            scope.tenant,
            scope.principal,
            scope.surface,
            scope.owner_tenant,
            &scope.labels,
            scope.task_types.as_deref(),
            scope.work_context,
            after.map(|cursor| cursor.created_at),
            after.map(|cursor| cursor.task_id.as_uuid()),
            i64::try_from(limit + 1).expect("a bounded page limit fits i64"),
        )
        .fetch_all(self.runtime.pool())
        .await?;
        let has_more = rows.len() > limit;
        let items = rows
            .into_iter()
            .take(limit)
            .map(TaskSnapshot::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = has_more
            .then(|| items.last())
            .flatten()
            .map(|last| TaskPageCursor {
                created_at: last.created_at,
                task_id: last.task_id,
            });
        Ok(TaskPage { items, next_cursor })
    }

    /// Cancel the caller's Task. An unowned Task is cancelled at once; an owned one
    /// records the request for its worker. A finished Task is returned unchanged.
    pub async fn cancel(&self, task: TaskId) -> Result<TaskSnapshot, TaskError> {
        let task = crate::runtime::validate_task_id(task)?;
        for _ in 0..crate::runtime::MAX_ATTEMPTS {
            let scope = self.scope();
            let row = sqlx::query_file_as!(
                TaskRow,
                "queries/owner/cancel.sql",
                scope.server,
                scope.tenant,
                scope.principal,
                scope.surface,
                scope.owner_tenant,
                &scope.labels,
                scope.task_types.as_deref(),
                scope.work_context,
                task.as_uuid(),
            )
            .fetch_optional(self.runtime.pool())
            .await?;
            let current = match row {
                Some(row) => TaskSnapshot::try_from(row)?,
                None => self.get(task).await?.ok_or(TaskError::NotFound(task))?,
            };
            if current.is_terminal() || current.status == TaskStatus::CancelRequested {
                self.runtime.stop_local_worker(&current).await;
                return Ok(current);
            }
        }
        Err(TaskError::Conflict(task))
    }

    /// Answer the caller's outstanding inputs. Each answer commits separately; an
    /// answer to an unknown or answered key is ignored.
    pub async fn submit_input_responses(
        &self,
        task: TaskId,
        responses: InputResponses,
    ) -> Result<TaskInputSubmission, TaskError> {
        let current = self.get(task).await?.ok_or(TaskError::NotFound(task))?;
        accepts_input(current.status)?;
        let mut submission = TaskInputSubmission::default();
        for (key, response) in responses {
            validate_input_key(&key)?;
            let scope = self.scope();
            let answer = sqlx::query_file!(
                "queries/owner/answer_input.sql",
                scope.server,
                scope.tenant,
                scope.principal,
                scope.surface,
                scope.owner_tenant,
                &scope.labels,
                scope.task_types.as_deref(),
                scope.work_context,
                task.as_uuid(),
                key,
                object(response),
            )
            .fetch_one(self.runtime.pool())
            .await?;
            if !answer.admitted {
                return Err(match self.get(task).await? {
                    Some(current) => accepts_input(current.status)
                        .err()
                        .unwrap_or(TaskError::Conflict(task)),
                    None => TaskError::NotFound(task),
                });
            }
            if answer.accepted {
                submission.accepted += 1;
            } else {
                submission.ignored += 1;
            }
        }
        Ok(submission)
    }
}
