//! Process instances: a row in `kernel.instance` and a session advisory lock that proves
//! the process alive, both held on one dedicated connection.
use std::{fmt, time::Duration};

use sqlx::{Connection, PgConnection};
use tokio::{
    sync::{oneshot, watch},
    task::JoinHandle,
    time::MissedTickBehavior,
};
use tokio_util::sync::CancellationToken;
use veoveo_types::ServerSlug;

use crate::{PlatformStore, StoreError, names::check_server_name};

/// How often an instance checks its lock connection.
pub const PING_INTERVAL: Duration = Duration::from_secs(5);
/// How long one check may take before the instance counts its connection as lost.
pub const PING_TIMEOUT: Duration = Duration::from_secs(5);
/// How often an instance reaps instances whose locks are free; it also reaps at start.
pub const REAP_INTERVAL: Duration = Duration::from_secs(15);

/// The identifier of a row in `kernel.instance`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, sqlx::Type)]
#[sqlx(transparent)]
pub struct InstanceId(i32);

impl InstanceId {
    pub const fn get(self) -> i32 {
        self.0
    }
}

impl fmt::Display for InstanceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The outcome of the most recent reap: how many rounds this instance has run and how
/// many dead instances the last round deleted. Workers that claim released work watch
/// it and claim again after each round.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReapRound {
    pub round: u64,
    pub reaped: u32,
}

/// A running process that owns work. Its lock connection lives in a background task
/// that checks the connection every [`PING_INTERVAL`] and reaps dead instances every
/// [`REAP_INTERVAL`]. When the connection fails the lock is gone, so the instance
/// reports itself lost and the process must stop acting on the work it owned.
pub struct Instance {
    id: InstanceId,
    lost: CancellationToken,
    reaps: watch::Receiver<ReapRound>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), StoreError>>>,
}

impl Instance {
    /// Record this process as an instance of `server` and take its lock. A kernel
    /// component passes its own name, such as `gateway`; a registered server passes its
    /// slug, which row-level security requires its login to own.
    pub async fn start(store: &PlatformStore, server: &ServerSlug) -> Result<Self, StoreError> {
        check_server_name(server)?;
        let mut connection = store.dedicated_connection().await?;
        let id = sqlx::query_file_scalar!("src/instance/start_instance.sql", server.as_str())
            .fetch_one(&mut connection)
            .await?;
        let lost = CancellationToken::new();
        let (reaps_sender, reaps) = watch::channel(ReapRound::default());
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(hold(connection, id, stopped, lost.clone(), reaps_sender));
        Ok(Self {
            id,
            lost,
            reaps,
            stop: Some(stop),
            task: Some(task),
        })
    }

    pub fn id(&self) -> InstanceId {
        self.id
    }

    /// Resolves once the lock connection is lost.
    pub async fn lost(&self) {
        self.lost.cancelled().await;
    }

    pub fn is_lost(&self) -> bool {
        self.lost.is_cancelled()
    }

    /// The reap rounds this instance runs.
    pub fn reaps(&self) -> watch::Receiver<ReapRound> {
        self.reaps.clone()
    }

    /// Delete this instance's row and close its lock connection. An instance dropped
    /// without shutdown closes the connection, and another instance reaps the row.
    pub async fn shutdown(mut self) -> Result<(), StoreError> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        match self.task.take() {
            Some(task) => task.await.unwrap_or(Err(StoreError::InstanceLost)),
            None => Ok(()),
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl fmt::Debug for Instance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Instance")
            .field("id", &self.id)
            .field("lost", &self.is_lost())
            .finish_non_exhaustive()
    }
}

async fn hold(
    mut connection: PgConnection,
    id: InstanceId,
    mut stopped: oneshot::Receiver<()>,
    lost: CancellationToken,
    reaps: watch::Sender<ReapRound>,
) -> Result<(), StoreError> {
    let reap_every = (REAP_INTERVAL.as_secs() / PING_INTERVAL.as_secs()).max(1);
    let mut ticker = tokio::time::interval(PING_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut tick = 0u64;
    loop {
        tokio::select! {
            _ = &mut stopped => {
                let deleted = sqlx::query_file!("src/instance/stop_instance.sql", id as InstanceId)
                    .execute(&mut connection)
                    .await;
                let closed = connection.close().await;
                deleted?;
                closed?;
                return Ok(());
            }
            _ = ticker.tick() => {
                let reap = tick.is_multiple_of(reap_every);
                tick += 1;
                let checked = tokio::time::timeout(PING_TIMEOUT, async {
                    if reap {
                        reap_instances(&mut connection).await.map(Some)
                    } else {
                        connection.ping().await.map(|()| None).map_err(StoreError::from)
                    }
                })
                .await;
                match checked {
                    Ok(Ok(Some(reaped))) => reaps.send_modify(|round| {
                        round.round += 1;
                        round.reaped = reaped;
                    }),
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(instance = %id, %error, "instance lock connection lost");
                        lost.cancel();
                        return Err(StoreError::InstanceLost);
                    }
                    Err(_) => {
                        tracing::warn!(instance = %id, "instance lock connection stopped answering");
                        lost.cancel();
                        return Err(StoreError::InstanceLost);
                    }
                }
            }
        }
    }
}

async fn reap_instances(connection: &mut PgConnection) -> Result<u32, StoreError> {
    let reaped = sqlx::query_file_scalar!("src/instance/reap_instances.sql")
        .fetch_one(connection)
        .await?;
    Ok(u32::try_from(reaped).unwrap_or_default())
}

impl PlatformStore {
    /// Whether some session holds the instance's lock.
    pub async fn instance_alive(&self, id: InstanceId) -> Result<bool, StoreError> {
        Ok(
            sqlx::query_file_scalar!("src/instance/instance_alive.sql", id as InstanceId)
                .fetch_one(self.pool())
                .await?,
        )
    }

    /// Delete the rows of instances whose locks are free; returns how many it deleted.
    pub async fn reap_instances(&self) -> Result<u32, StoreError> {
        let mut connection = self.pool().acquire().await?;
        reap_instances(&mut connection).await
    }
}
