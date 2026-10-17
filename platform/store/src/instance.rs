//! Process instances: a row in `kernel.instance` and a session advisory lock that proves
//! the process alive, both held on one dedicated connection. When that connection fails,
//! the instance reconnects. It takes its lock back while its row is there and keeps the
//! work it owns. A reaper deletes the row only once the lock has stayed free for the
//! instance grace, and moves that work to recovery; an instance that comes back after
//! that starts again under a new id.
use std::{
    fmt,
    sync::{Mutex, PoisonError},
    time::Duration,
};

use sqlx::{Connection, PgConnection, postgres::types::PgInterval};
use tokio::{
    sync::{Mutex as AsyncMutex, oneshot, watch},
    task::JoinHandle,
    time::MissedTickBehavior,
};
use tokio_util::sync::CancellationToken;
use veoveo_types::ServerSlug;

use crate::{PlatformStore, StoreError, names::check_server_name};

/// How often an instance checks its lock connection and its row.
pub const PING_INTERVAL: Duration = Duration::from_secs(5);
/// How long one check may take before the instance counts its connection as lost.
pub const PING_TIMEOUT: Duration = Duration::from_secs(5);
/// How often an instance reaps instances whose locks have stayed free for the grace; it
/// also reaps at start.
pub const REAP_INTERVAL: Duration = Duration::from_secs(15);
/// How long an instance's lock stays free before a reaper deletes the instance and moves
/// its work to recovery, unless the store configuration sets another grace. It covers an
/// instance noticing a lost connection, which takes up to [`PING_INTERVAL`] plus
/// [`PING_TIMEOUT`], and several reconnection attempts. It stays shorter than the
/// Hub's 60-second registration grace, so a crashed server's Tasks reach recovery
/// before its registration goes, within this grace and two [`REAP_INTERVAL`]s of the
/// crash while any instance runs.
pub const INSTANCE_GRACE: InstanceGrace = InstanceGrace::from_secs(30);

/// How long an instance's lock stays free before a reaper deletes the instance, in whole
/// milliseconds, so every value is an interval PostgreSQL holds exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstanceGrace {
    millis: u64,
}

impl InstanceGrace {
    /// Delete an instance as soon as its lock is free.
    pub const ZERO: Self = Self { millis: 0 };

    pub const fn from_secs(secs: u32) -> Self {
        Self {
            millis: secs as u64 * 1000,
        }
    }

    pub const fn from_millis(millis: u32) -> Self {
        Self {
            millis: millis as u64,
        }
    }

    pub const fn as_duration(self) -> Duration {
        Duration::from_millis(self.millis)
    }

    fn interval(self) -> PgInterval {
        PgInterval {
            months: 0,
            days: 0,
            // At most u32::MAX seconds, which is far inside i64 microseconds.
            microseconds: (self.millis * 1000) as i64,
        }
    }
}
/// The wait after the first failed attempt to reconnect a lost lock connection. Each
/// further failure doubles it, up to [`RECONNECT_MAX_DELAY`].
pub const RECONNECT_FIRST_DELAY: Duration = Duration::from_millis(100);
/// The longest wait between attempts to reconnect a lost lock connection.
pub const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(5);

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

/// Whether an instance holds its lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstancePhase {
    /// The process holds the lock of the state's id.
    Live,
    /// The lock connection failed and the instance is reconnecting. Its work stays its
    /// own until its lock has stayed free for the instance grace and a reaper deletes
    /// the row.
    Reconnecting,
    /// The instance ended: it shut down, or the database refused its login, as it does
    /// once the Hub removed the server's registration.
    Ended,
}

/// Where an instance stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstanceState {
    /// The id the instance holds or reconnects to, or held last.
    pub id: InstanceId,
    /// 0 for the instance the process started, and one more each time a reaper deleted
    /// the row while the connection was down and the instance started under a new id.
    /// Work claimed in an earlier generation belongs to that generation's recovery.
    pub generation: u64,
    pub phase: InstancePhase,
}

/// A running process that owns work. Its lock connection lives in a background task
/// that checks the connection and the instance's row every [`PING_INTERVAL`] and, every
/// [`REAP_INTERVAL`], reaps instances whose locks have stayed free for the store's
/// instance grace. A failed connection is reconnected with
/// backoff from [`RECONNECT_FIRST_DELAY`] to [`RECONNECT_MAX_DELAY`]: the instance takes
/// its lock back while its row is there, and starts a new generation under a new id
/// once a reaper deleted the row. The instance ends only on shutdown or when the
/// database refuses its login; the process must then stop acting on the work it owned.
pub struct Instance {
    state: watch::Receiver<InstanceState>,
    lost: CancellationToken,
    reaps: watch::Receiver<ReapRound>,
    stop: Mutex<Option<oneshot::Sender<()>>>,
    task: AsyncMutex<Option<JoinHandle<Result<(), StoreError>>>>,
}

impl Instance {
    /// Record this process as an instance of `server` and take its lock. A kernel
    /// component passes its own name, such as `hub`; a registered server passes its
    /// slug, which row-level security requires its login to own.
    pub async fn start(store: &PlatformStore, server: &ServerSlug) -> Result<Self, StoreError> {
        check_server_name(server)?;
        let mut connection = store.dedicated_connection().await?;
        let id = start_instance(&mut connection, server).await?;
        let lost = CancellationToken::new();
        let (reaps_sender, reaps) = watch::channel(ReapRound::default());
        let (state_sender, state) = watch::channel(InstanceState {
            id,
            generation: 0,
            phase: InstancePhase::Live,
        });
        let (stop, stopped) = oneshot::channel();
        let holder = Holder {
            store: store.clone(),
            server: server.clone(),
            state: state_sender,
            lost: lost.clone(),
            reaps: reaps_sender,
        };
        let task = tokio::spawn(holder.hold(connection, stopped));
        Ok(Self {
            state,
            lost,
            reaps,
            stop: Mutex::new(Some(stop)),
            task: AsyncMutex::new(Some(task)),
        })
    }

    /// The id the instance holds or reconnects to now. Work is claimed under it.
    pub fn id(&self) -> InstanceId {
        self.state.borrow().id
    }

    /// The instance's id, generation and phase now.
    pub fn state(&self) -> InstanceState {
        *self.state.borrow()
    }

    /// The instance's states as they change.
    pub fn changes(&self) -> watch::Receiver<InstanceState> {
        self.state.clone()
    }

    /// Resolves once the instance ended: it shut down, or the database refused its login.
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

    /// Delete this instance's row and close its lock connection, which releases the
    /// work it owned at once. Every holder of a shared instance may call it; the first
    /// call stops the instance, and it reports itself lost from then on. An instance
    /// dropped without shutdown, or shut down while it reconnects, leaves its row, and
    /// another instance reaps it.
    pub async fn shutdown(&self) -> Result<(), StoreError> {
        let stop = self
            .stop
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(stop) = stop {
            let _ = stop.send(());
        }
        let task = self.task.lock().await.take();
        let stopped = match task {
            Some(task) => task.await.unwrap_or(Err(StoreError::InstanceLost)),
            None => Ok(()),
        };
        self.lost.cancel();
        stopped
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        if let Some(task) = self.task.get_mut().take() {
            task.abort();
        }
    }
}

impl fmt::Debug for Instance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Instance")
            .field("state", &self.state())
            .field("lost", &self.is_lost())
            .finish_non_exhaustive()
    }
}

/// The background task that holds an instance's lock connection.
struct Holder {
    store: PlatformStore,
    server: ServerSlug,
    state: watch::Sender<InstanceState>,
    lost: CancellationToken,
    reaps: watch::Sender<ReapRound>,
}

/// How a reconnection attempt ended.
enum Attempt {
    /// The process holds an instance again on this connection.
    Holding(PgConnection),
    /// The process's earlier session still held the lock, and the attempt ended it.
    Held,
}

impl Holder {
    async fn hold(
        self,
        connection: PgConnection,
        mut stopped: oneshot::Receiver<()>,
    ) -> Result<(), StoreError> {
        let held = self.run(connection, &mut stopped).await;
        self.state
            .send_modify(|state| state.phase = InstancePhase::Ended);
        self.lost.cancel();
        held
    }

    fn id(&self) -> InstanceId {
        self.state.borrow().id
    }

    async fn run(
        &self,
        mut connection: PgConnection,
        stopped: &mut oneshot::Receiver<()>,
    ) -> Result<(), StoreError> {
        let reap_every = (REAP_INTERVAL.as_secs() / PING_INTERVAL.as_secs()).max(1);
        let mut ticker = tokio::time::interval(PING_INTERVAL);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut tick = 0u64;
        loop {
            tokio::select! {
                _ = &mut *stopped => return self.stop(connection).await,
                _ = ticker.tick() => {
                    let reap = tick.is_multiple_of(reap_every);
                    tick += 1;
                    let id = self.id();
                    let grace = reap.then(|| self.store.config().instance_grace());
                    let checked =
                        tokio::time::timeout(PING_TIMEOUT, check(&mut connection, id, grace)).await;
                    match checked {
                        Ok(Ok(Check { reaped, present: true })) => {
                            if let Some(reaped) = reaped {
                                self.reaps.send_modify(|round| {
                                    round.round += 1;
                                    round.reaped = reaped;
                                });
                            }
                            continue;
                        }
                        Ok(Ok(Check { present: false, .. })) => tracing::warn!(
                            instance = %id,
                            "a reaper deleted the instance's row; it starts again"
                        ),
                        Ok(Err(error)) => tracing::warn!(
                            instance = %id,
                            %error,
                            "instance lock connection lost; reconnecting"
                        ),
                        Err(_) => tracing::warn!(
                            instance = %id,
                            "instance lock connection stopped answering; reconnecting"
                        ),
                    }
                    // Closing the old connection releases whatever lock it still holds.
                    drop(connection);
                    connection = match self.reconnect(stopped).await? {
                        Some(connection) => connection,
                        None => return Ok(()),
                    };
                    ticker.reset();
                }
            }
        }
    }

    async fn stop(&self, mut connection: PgConnection) -> Result<(), StoreError> {
        let stopped =
            sqlx::query_file_scalar!("src/instance/stop_instance.sql", self.id() as InstanceId)
                .fetch_one(&mut connection)
                .await;
        let closed = connection.close().await;
        stopped?;
        closed?;
        Ok(())
    }

    /// Reconnect until the process holds an instance again. Returns `None` when the
    /// instance is stopped first, and fails once the database refuses the login.
    async fn reconnect(
        &self,
        stopped: &mut oneshot::Receiver<()>,
    ) -> Result<Option<PgConnection>, StoreError> {
        self.state
            .send_modify(|state| state.phase = InstancePhase::Reconnecting);
        let mut delay = RECONNECT_FIRST_DELAY;
        let mut attempts = 0u32;
        loop {
            attempts = attempts.saturating_add(1);
            match self.attempt().await {
                Ok(Attempt::Holding(connection)) => return Ok(Some(connection)),
                // The earlier session ended just now, so the next attempt comes soon.
                Ok(Attempt::Held) => delay = RECONNECT_FIRST_DELAY,
                Err(error) if error.is_login_refused() => {
                    tracing::error!(
                        instance = %self.id(),
                        %error,
                        "the database refused the instance's login, so the server's registration is gone; the instance ends"
                    );
                    return Err(StoreError::InstanceLost);
                }
                Err(error) => tracing::warn!(
                    instance = %self.id(),
                    attempts,
                    %error,
                    "reconnecting the instance failed"
                ),
            }
            tokio::select! {
                _ = &mut *stopped => return Ok(None),
                () = tokio::time::sleep(delay) => {}
            }
            delay = (delay * 2).min(RECONNECT_MAX_DELAY);
        }
    }

    async fn attempt(&self) -> Result<Attempt, StoreError> {
        let mut connection = self.store.dedicated_connection().await?;
        let id = self.id();
        let outcome = within(
            "resuming a process instance",
            sqlx::query_file_scalar!("src/instance/resume_instance.sql", id as InstanceId)
                .fetch_one(&mut connection),
        )
        .await?;
        match outcome.as_str() {
            "resumed" => {
                self.state
                    .send_modify(|state| state.phase = InstancePhase::Live);
                tracing::info!(instance = %id, "the instance took its lock back");
                Ok(Attempt::Holding(connection))
            }
            "held" => Ok(Attempt::Held),
            "gone" => {
                let started = within(
                    "starting a process instance",
                    start_instance(&mut connection, &self.server),
                )
                .await?;
                self.state.send_modify(|state| {
                    state.id = started;
                    state.generation += 1;
                    state.phase = InstancePhase::Live;
                });
                tracing::warn!(
                    instance = %started,
                    previous = %id,
                    "a reaper moved the instance's work to recovery while its connection was down; it continues as a new instance"
                );
                Ok(Attempt::Holding(connection))
            }
            _ => Err(StoreError::InvalidStoredValue {
                column: "kernel.resume_instance",
            }),
        }
    }
}

/// What one check of the lock connection found.
struct Check {
    /// How many dead instances this check reaped, when it reaped.
    reaped: Option<u32>,
    /// Whether the instance's row is still there.
    present: bool,
}

/// Check the lock connection and the instance's row, and reap with `grace` when given.
async fn check(
    connection: &mut PgConnection,
    id: InstanceId,
    grace: Option<InstanceGrace>,
) -> Result<Check, StoreError> {
    let reaped = match grace {
        Some(grace) => Some(reap_instances(connection, grace).await?),
        None => None,
    };
    let present = sqlx::query_file_scalar!("src/instance/instance_present.sql", id as InstanceId)
        .fetch_one(connection)
        .await?;
    Ok(Check { reaped, present })
}

async fn start_instance(
    connection: &mut PgConnection,
    server: &ServerSlug,
) -> Result<InstanceId, StoreError> {
    Ok(
        sqlx::query_file_scalar!("src/instance/start_instance.sql", server.as_str())
            .fetch_one(connection)
            .await?,
    )
}

/// Run one statement of a reconnection under [`PING_TIMEOUT`].
async fn within<T, E: Into<StoreError>>(
    operation: &'static str,
    statement: impl Future<Output = Result<T, E>>,
) -> Result<T, StoreError> {
    match tokio::time::timeout(PING_TIMEOUT, statement).await {
        Ok(done) => done.map_err(Into::into),
        Err(_) => Err(StoreError::Timeout {
            operation,
            seconds: PING_TIMEOUT.as_secs(),
        }),
    }
}

async fn reap_instances(
    connection: &mut PgConnection,
    grace: InstanceGrace,
) -> Result<u32, StoreError> {
    let reaped = sqlx::query_file_scalar!("src/instance/reap_instances.sql", grace.interval())
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

    /// Mark the instances whose locks are free and delete those whose locks have stayed
    /// free for the configured instance grace; returns how many it deleted. With a grace
    /// of zero it deletes every instance whose lock is free now.
    pub async fn reap_instances(&self) -> Result<u32, StoreError> {
        let mut connection = self.pool().acquire().await?;
        reap_instances(&mut connection, self.config().instance_grace()).await
    }
}
