//! Caller subscriptions deliver the current state of watched Tasks. A notification
//! wakes the reader; it rereads the Task under the caller's selection and delivers it
//! when its revision changed.
use std::{
    collections::{BTreeMap, BTreeSet},
    pin::Pin,
};

use futures::Stream;
use veoveo_types::TaskId;

use crate::{
    OwnerTaskQuery, TaskError, TaskSnapshot,
    row::{TaskRow, columns::*},
};

/// The most Tasks one subscription watches.
const MAX_SUBSCRIBED: usize = 256;

pub type TaskUpdateStream =
    Pin<Box<dyn Stream<Item = Result<TaskSnapshot, TaskError>> + Send + 'static>>;

pub struct OwnerTaskSubscription {
    /// The requested Tasks that exist and belong to the caller.
    pub accepted_task_ids: Vec<TaskId>,
    /// Current state of the accepted Tasks: first each Task's state at subscription,
    /// then every later revision the reader observes. Intermediate revisions may
    /// coalesce.
    pub updates: TaskUpdateStream,
}

impl OwnerTaskQuery {
    /// Watch at most 256 of the caller's Tasks. Every delivery rereads the Task under
    /// this query's current selection; a listener reconnect rereads all of them.
    pub async fn subscribe(&self, ids: &[TaskId]) -> Result<OwnerTaskSubscription, TaskError> {
        if ids.len() > MAX_SUBSCRIBED {
            return Err(TaskError::InvalidPageQuery);
        }
        let ids = ids
            .iter()
            .copied()
            .map(crate::runtime::validate_task_id)
            .collect::<Result<BTreeSet<_>, _>>()?;
        if ids.is_empty() {
            return Ok(OwnerTaskSubscription {
                accepted_task_ids: Vec::new(),
                updates: Box::pin(futures::stream::pending()),
            });
        }
        let mut signals = self.runtime.signals().await?;
        let initial = self.current(&ids).await?;
        let accepted_task_ids = initial
            .iter()
            .map(|snapshot| snapshot.task_id)
            .collect::<Vec<_>>();
        if accepted_task_ids.is_empty() {
            return Ok(OwnerTaskSubscription {
                accepted_task_ids,
                updates: Box::pin(futures::stream::pending()),
            });
        }
        let watched = accepted_task_ids.iter().copied().collect::<BTreeSet<_>>();
        let query = self.clone();
        let updates = Box::pin(async_stream::try_stream! {
            let mut delivered = BTreeMap::new();
            for snapshot in initial {
                delivered.insert(snapshot.task_id, snapshot.revision);
                yield snapshot;
            }
            while let Some(wake) = signals.next().await {
                let changed = if wake.reconcile {
                    watched.clone()
                } else {
                    wake.changed.intersection(&watched).copied().collect()
                };
                for snapshot in query.current(&changed).await? {
                    if delivered.insert(snapshot.task_id, snapshot.revision)
                        != Some(snapshot.revision)
                    {
                        yield snapshot;
                    }
                }
            }
        });
        Ok(OwnerTaskSubscription {
            accepted_task_ids,
            updates,
        })
    }

    /// Current state of the caller's Tasks among `ids`.
    async fn current(&self, ids: &BTreeSet<TaskId>) -> Result<Vec<TaskSnapshot>, TaskError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let scope = self.scope();
        sqlx::query_file_as!(
            TaskRow,
            "queries/owner/current.sql",
            scope.server,
            scope.tenant,
            scope.principal,
            scope.surface,
            scope.owner_tenant,
            &scope.labels,
            scope.task_types.as_deref(),
            scope.work_context,
            &ids.iter().map(|id| id.as_uuid()).collect::<Vec<_>>(),
        )
        .fetch_all(self.runtime.pool())
        .await?
        .into_iter()
        .map(TaskSnapshot::try_from)
        .collect()
    }
}
