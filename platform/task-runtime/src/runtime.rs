//! The Task runtime of one server process.
use std::{collections::HashMap, sync::Arc, time::Duration};

use sqlx::PgPool;
use tokio::{sync::Mutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use veoveo_platform_store::{Channel, Instance, InstanceId, PlatformStore, StoreError};
use veoveo_types::{ServerSlug, TaskId};

use crate::{
    TaskError, TaskOwner, TaskPayloadState, TaskRetentionPin, TaskRuntimeConfig, TaskSnapshot,
    TaskStatus,
    operations::due::Due,
    retention::Retention,
    row::{TaskRow, columns::*},
    signals::{SignalHub, TaskSignals},
};

/// Bounded retries of a control write that raced a concurrent change.
pub(crate) const MAX_ATTEMPTS: usize = 8;

struct Worker {
    cancellation: CancellationToken,
    join: JoinHandle<()>,
    /// The instance generation whose claim the worker runs.
    generation: u64,
}

/// Durable Tasks of one server, executed by one process instance.
///
/// The instance owns every Task this runtime claims. When the process ends, the kernel
/// reaper deletes its instance row and the Tasks move to their recovery state; a worker
/// write from a reaped instance matches no row. When the reaper deleted the row while the
/// instance's connection was down, the instance continues as a new generation under a
/// new id, and the runtime stops the workers of earlier generations before it claims
/// again, so no Task runs twice in one process.
#[derive(Clone)]
pub struct TaskRuntime {
    store: PlatformStore,
    server: ServerSlug,
    instance: Arc<Instance>,
    workers: Arc<Mutex<HashMap<TaskId, Worker>>>,
    /// The instance generation of each claim whose worker is not registered yet.
    claimed: Arc<std::sync::Mutex<HashMap<TaskId, u64>>>,
    signals: Arc<SignalHub>,
    _retention: Arc<Retention>,
    _due: Arc<Due>,
}

impl TaskRuntime {
    /// Connect to the installation database and start this process's instance.
    pub async fn connect(config: TaskRuntimeConfig, server: ServerSlug) -> Result<Self, TaskError> {
        let store = PlatformStore::connect(config.store_config().clone()).await?;
        let instance = Instance::start(&store, &server).await?;
        Ok(Self::new(store, server, Arc::new(instance)))
    }

    /// A runtime of `server` whose claims belong to `instance`. It prunes the server's
    /// expired Tasks every [`RETENTION_INTERVAL`](crate::RETENTION_INTERVAL) and queues
    /// Tasks whose operations fall due, at least every
    /// [`DUE_INTERVAL`](crate::DUE_INTERVAL), until its last clone is dropped or the
    /// instance is lost, so it must be created inside a Tokio runtime.
    pub fn new(store: PlatformStore, server: ServerSlug, instance: Arc<Instance>) -> Self {
        Self::with_retention_interval(store, server, instance, crate::RETENTION_INTERVAL)
    }

    pub(crate) fn with_retention_interval(
        store: PlatformStore,
        server: ServerSlug,
        instance: Arc<Instance>,
        interval: Duration,
    ) -> Self {
        let retention = Retention::spawn(store.clone(), server.clone(), &instance, interval);
        let due = Due::spawn(
            store.clone(),
            server.clone(),
            &instance,
            crate::operations::DUE_INTERVAL,
        );
        Self {
            store,
            server,
            instance,
            workers: Arc::default(),
            claimed: Arc::default(),
            signals: Arc::default(),
            _retention: Arc::new(retention),
            _due: Arc::new(due),
        }
    }

    pub fn platform_store(&self) -> &PlatformStore {
        &self.store
    }

    pub fn server(&self) -> &ServerSlug {
        &self.server
    }

    /// The process instance that owns this runtime's claims.
    pub fn instance(&self) -> &Arc<Instance> {
        &self.instance
    }

    pub(crate) fn instance_id(&self) -> InstanceId {
        self.instance.id()
    }

    /// Fail once the instance ended, so the process stops taking work.
    pub(crate) fn check_instance(&self) -> Result<(), TaskError> {
        if self.instance.is_lost() {
            return Err(StoreError::InstanceLost.into());
        }
        Ok(())
    }

    /// The instance id a claim records and its generation. When the instance continued
    /// as a new generation, a reaper moved the Tasks of the earlier one to recovery, so
    /// their local workers no longer own them: the runtime stops those workers and waits
    /// for them before this process can claim one of those Tasks again.
    pub(crate) async fn claim_instance(&self) -> Result<(InstanceId, u64), TaskError> {
        self.check_instance()?;
        let state = self.instance.state();
        let mut workers = self.workers.lock().await;
        let earlier: Vec<TaskId> = workers
            .iter()
            .filter(|(_, worker)| worker.generation < state.generation)
            .map(|(task_id, _)| *task_id)
            .collect();
        for task_id in earlier {
            if let Some(worker) = workers.remove(&task_id) {
                tracing::warn!(
                    task = %task_id,
                    instance = %state.id,
                    "stopping the worker of a Task an earlier instance generation owned"
                );
                worker.join.abort();
                let _ = worker.join.await;
            }
        }
        Ok((state.id, state.generation))
    }

    /// Remember the generation of a claim until its worker is registered.
    pub(crate) fn claimed(&self, snapshot: TaskSnapshot, generation: u64) -> TaskSnapshot {
        self.claimed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(snapshot.task_id, generation);
        snapshot
    }

    pub(crate) fn pool(&self) -> &PgPool {
        self.store.pool()
    }

    /// The channel on which the changes of this server's Tasks, and only those, arrive:
    /// `tasks.<server>`. It fails for a server name too long for a PostgreSQL channel.
    pub fn channel(&self) -> Result<Channel, TaskError> {
        Ok(Channel::TASKS.qualified(self.server.as_str())?)
    }

    /// Read this server's channel through this runtime's shared listener.
    pub(crate) async fn signals(&self) -> Result<TaskSignals, TaskError> {
        self.signals.subscribe(&self.store, self.channel()?).await
    }

    /// This server's Task, without a caller selection. Callers read through `for_owner`.
    /// ```compile_fail
    /// use veoveo_task_runtime::TaskRuntime;
    /// async fn raw_identity(runtime: &TaskRuntime) {
    ///     runtime.get("01983da0-0000-7000-8000-000000000001").await;
    /// }
    /// ```
    pub async fn get(&self, task_id: TaskId) -> Result<Option<TaskSnapshot>, TaskError> {
        let task_id = validate_task_id(task_id)?;
        sqlx::query_file_as!(
            TaskRow,
            "queries/get.sql",
            task_id.as_uuid(),
            self.server.as_str(),
        )
        .fetch_optional(self.pool())
        .await?
        .map(TaskSnapshot::try_from)
        .transpose()
    }

    /// The owner and authority retained with this server's Task.
    pub async fn owner(&self, task_id: TaskId) -> Result<Option<TaskOwner>, TaskError> {
        Ok(self.get(task_id).await?.map(|snapshot| snapshot.owner))
    }

    /// Request cancellation of this server's Task without a caller selection. An unowned
    /// Task is cancelled at once; an owned one records the request for its worker. A
    /// finished Task is returned unchanged.
    pub async fn cancel(&self, task_id: TaskId) -> Result<TaskSnapshot, TaskError> {
        let task_id = validate_task_id(task_id)?;
        for _ in 0..MAX_ATTEMPTS {
            let row = sqlx::query_file_as!(
                TaskRow,
                "queries/cancel.sql",
                task_id.as_uuid(),
                self.server.as_str(),
            )
            .fetch_optional(self.pool())
            .await?;
            let current = match row {
                Some(row) => TaskSnapshot::try_from(row)?,
                None => self
                    .get(task_id)
                    .await?
                    .ok_or(TaskError::NotFound(task_id))?,
            };
            if current.is_terminal() || current.status == TaskStatus::CancelRequested {
                self.stop_local_worker(&current).await;
                return Ok(current);
            }
        }
        Err(TaskError::Conflict(task_id))
    }

    pub async fn is_cancel_requested(&self, task_id: TaskId) -> Result<bool, TaskError> {
        Ok(self
            .get(task_id)
            .await?
            .is_some_and(|snapshot| snapshot.status == TaskStatus::CancelRequested))
    }

    pub async fn payload_state(&self, task_id: TaskId) -> Result<TaskPayloadState, TaskError> {
        Ok(self.get(task_id).await?.into())
    }

    /// Track a local worker, so a cancellation through this runtime stops it at once.
    pub async fn register_worker(
        &self,
        task_id: TaskId,
        cancellation: CancellationToken,
        join: JoinHandle<()>,
    ) -> Result<(), TaskError> {
        let task_id = validate_task_id(task_id)?;
        let generation = self
            .claimed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&task_id)
            .unwrap_or_else(|| self.instance.state().generation);
        self.workers.lock().await.insert(
            task_id,
            Worker {
                cancellation,
                join,
                generation,
            },
        );
        Ok(())
    }

    /// Forget local workers that finished.
    pub async fn reap_workers(&self) {
        self.workers
            .lock()
            .await
            .retain(|_, worker| !worker.join.is_finished());
    }

    pub(crate) async fn stop_local_worker(&self, snapshot: &TaskSnapshot) {
        if snapshot.status == TaskStatus::CancelRequested
            && let Some(worker) = self.workers.lock().await.get(&snapshot.task_id)
        {
            worker.cancellation.cancel();
        }
    }

    /// Release one consumer's retention pin after it acknowledged the result. Repeating
    /// an acknowledgement changes nothing.
    pub async fn acknowledge_retention_pin(
        &self,
        task_id: TaskId,
        pin: &TaskRetentionPin,
    ) -> Result<TaskSnapshot, TaskError> {
        let task_id = validate_task_id(task_id)?;
        let row = sqlx::query_file_as!(
            TaskRow,
            "queries/acknowledge_retention_pin.sql",
            task_id.as_uuid(),
            self.server.as_str(),
            pin.as_str(),
        )
        .fetch_optional(self.pool())
        .await?;
        match row {
            Some(row) => TaskSnapshot::try_from(row),
            None => self.get(task_id).await?.ok_or(TaskError::NotFound(task_id)),
        }
    }
}

/// Native Task lookups admit RFC 9562 UUIDv7 identities only.
pub(crate) fn validate_task_id(task_id: TaskId) -> Result<TaskId, TaskError> {
    let uuid = task_id.as_uuid();
    if uuid.get_version_num() != 7 || uuid.get_variant() != uuid::Variant::RFC4122 {
        return Err(TaskError::invalid_record("task id must be a UUIDv7"));
    }
    Ok(task_id)
}

/// Admit an external Task handle as a native UUIDv7 identity.
pub fn parse_task_id(value: &str) -> Result<TaskId, TaskError> {
    let task_id = value
        .parse::<TaskId>()
        .map_err(|error| TaskError::invalid_record(error.to_string()))?;
    validate_task_id(task_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_task_ids_must_be_uuid_v7() {
        assert!(parse_task_id(&uuid::Uuid::now_v7().to_string()).is_ok());
        assert!(validate_task_id(TaskId::new()).is_ok());
        assert!(parse_task_id("01983da0-0000-7000-0000-000000000001").is_err());
        assert!(matches!(
            parse_task_id(&uuid::Uuid::new_v4().to_string()),
            Err(TaskError::InvalidRecord(message)) if message == "task id must be a UUIDv7"
        ));
    }
}
