//! The Gateway's audit service: a standby until it holds the sealer role, then the
//! sealer with export and retention. Failures are observable through its health.
use crate::{
    integrity::AuditSigningKey,
    sealer::{AuditSealer, SealError},
    store::{AuditStore, AuditStoreError},
};
use std::{
    num::NonZeroU32,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::watch;

/// How often a waiting standby reports that it is alive.
const STANDBY_HEARTBEAT: Duration = Duration::from_secs(10);
/// The most blocks one retention pass retires.
const RETENTION_BATCH: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditHealthState {
    Starting,
    Active,
    Standby,
    Recovering,
    Failed,
    Stopped,
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct HealthUpdate {
    state: AuditHealthState,
    checked: Instant,
}
impl HealthUpdate {
    pub(crate) fn now(state: AuditHealthState) -> Self {
        Self {
            state,
            checked: Instant::now(),
        }
    }
}
#[derive(Debug, Clone)]
pub struct AuditHealth(watch::Receiver<HealthUpdate>);
impl AuditHealth {
    pub fn ready(&self) -> bool {
        let health = *self.0.borrow();
        matches!(
            health.state,
            AuditHealthState::Active | AuditHealthState::Standby
        ) && health.checked.elapsed() < Duration::from_secs(20)
    }
    pub fn state(&self) -> AuditHealthState {
        self.0.borrow().state
    }
    pub async fn changed(&mut self) -> Result<(), watch::error::RecvError> {
        self.0.changed().await
    }
}
#[derive(Debug, thiserror::Error)]
pub enum AuditServiceError {
    #[error("audit retention exceeds the supported UTC date range")]
    InvalidRetention,
    #[error(transparent)]
    Export(#[from] crate::export::ExportError),
    #[error(transparent)]
    Seal(#[from] SealError),
    #[error("audit service worker stopped unexpectedly")]
    Worker,
    #[error("audit service shutdown deadline exceeded; sealing is incomplete")]
    ShutdownDeadline,
}
pub struct AuditService {
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<Result<(), AuditServiceError>>,
    health: AuditHealth,
}
impl AuditService {
    pub fn start(
        audit: AuditStore,
        key: Arc<AuditSigningKey>,
        retention_days: NonZeroU32,
        exports: crate::export::AuditExportConfig,
    ) -> Result<Self, AuditServiceError> {
        chrono::Utc::now()
            .checked_sub_signed(chrono::TimeDelta::days(i64::from(retention_days.get())))
            .ok_or(AuditServiceError::InvalidRetention)?;
        let exporter = crate::export::AuditExporter::new(exports)?;
        let (stop, shutdown) = watch::channel(false);
        let (health, receiver) = watch::channel(HealthUpdate::now(AuditHealthState::Starting));
        let task = tokio::spawn(async move {
            let maintenance = Maintenance {
                audit: audit.clone(),
                retention_days,
                destinations: exporter.destination_ids(),
                exporter: Arc::new(exporter),
            };
            let result = supervise(audit, key, &maintenance, shutdown, &health).await;
            health.send_replace(HealthUpdate::now(if result.is_ok() {
                AuditHealthState::Stopped
            } else {
                AuditHealthState::Failed
            }));
            result
        });
        Ok(Self {
            stop,
            task,
            health: AuditHealth(receiver),
        })
    }
    pub fn health(&self) -> AuditHealth {
        self.health.clone()
    }
    /// Call after the request writer drains so its final committed records are sealed.
    pub async fn shutdown(mut self, timeout: Duration) -> Result<(), AuditServiceError> {
        self.stop.send_replace(true);
        match tokio::time::timeout(timeout, &mut self.task).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(AuditServiceError::Worker),
            Err(_) => {
                self.task.abort();
                let _ = (&mut self.task).await;
                Err(AuditServiceError::ShutdownDeadline)
            }
        }
    }
}
impl Drop for AuditService {
    fn drop(&mut self) {
        self.stop.send_replace(true);
        if !self.task.is_finished() {
            tracing::error!("audit service dropped before its shutdown completed");
            self.task.abort();
        }
    }
}

/// Failures that waiting for the role again cannot repair.
fn fatal(error: &SealError) -> bool {
    matches!(
        error,
        SealError::Export(_)
            | SealError::Integrity(_)
            | SealError::Exporter
            | SealError::Store(
                AuditStoreError::Integrity
                    | AuditStoreError::ExportRejected
                    | AuditStoreError::IdentityConflict
                    | AuditStoreError::BatchLimit
                    | AuditStoreError::Target(_)
                    | AuditStoreError::Validation(_),
            )
    )
}

async fn stopping(shutdown: &mut watch::Receiver<bool>) {
    while !*shutdown.borrow() {
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

async fn supervise(
    audit: AuditStore,
    key: Arc<AuditSigningKey>,
    maintenance: &Maintenance,
    mut shutdown: watch::Receiver<bool>,
    health: &watch::Sender<HealthUpdate>,
) -> Result<(), AuditServiceError> {
    let mut delay = Duration::from_secs(1);
    loop {
        health.send_replace(HealthUpdate::now(AuditHealthState::Standby));
        let acquire = AuditSealer::acquire(audit.clone(), key.clone());
        tokio::pin!(acquire);
        let mut heartbeat = tokio::time::interval(STANDBY_HEARTBEAT);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let acquired = loop {
            tokio::select! {
                acquired = &mut acquire => break acquired,
                // A standby that stops has no committed work of its own to seal.
                () = stopping(&mut shutdown) => return Ok(()),
                _ = heartbeat.tick() => {
                    health.send_replace(HealthUpdate::now(AuditHealthState::Standby));
                }
            }
        };
        let error = match acquired {
            Ok(sealer) => {
                let sealer = Arc::new(sealer);
                let result = sealer
                    .clone()
                    .run(shutdown.clone(), health, maintenance)
                    .await;
                if let Ok(sealer) = Arc::try_unwrap(sealer)
                    && let Err(error) = sealer.release().await
                {
                    tracing::warn!(%error, "audit sealer role connection did not close cleanly");
                }
                match result {
                    Ok(()) => return Ok(()),
                    Err(error) if fatal(&error) => {
                        tracing::error!(%error, "audit integrity service failed; operator recovery required");
                        return Err(error.into());
                    }
                    Err(error) => error,
                }
            }
            Err(error) => error,
        };
        health.send_replace(HealthUpdate::now(AuditHealthState::Recovering));
        tracing::warn!(%error, "audit sealer waiting for its role again");
        let pause = delay;
        delay = (delay * 2).min(Duration::from_secs(10));
        tokio::select! {
            () = tokio::time::sleep(pause) => {}
            // A sealer that failed has not sealed its pending records.
            () = stopping(&mut shutdown) => return Err(error.into()),
        }
    }
}

pub(crate) struct Maintenance {
    audit: AuditStore,
    retention_days: NonZeroU32,
    pub(crate) exporter: Arc<crate::export::AuditExporter>,
    destinations: Vec<veoveo_audit_contract::AuditDestinationId>,
}
impl Maintenance {
    /// Retire up to 128 expired blocks within a two-second budget.
    pub(crate) async fn run(&self, sealer: &AuditSealer) -> Result<(), SealError> {
        let cutoff = chrono::Utc::now()
            .checked_sub_signed(chrono::TimeDelta::days(i64::from(
                self.retention_days.get(),
            )))
            .ok_or(AuditStoreError::Integrity)?;
        let work = async {
            let mut retired = 0usize;
            while retired < RETENTION_BATCH {
                let blocks = self
                    .audit
                    .retention_candidates(cutoff, &self.destinations)
                    .await?;
                let before = retired;
                for block in blocks {
                    if retired == RETENTION_BATCH {
                        break;
                    }
                    if sealer.retire(&block, cutoff, &self.destinations).await? {
                        retired += 1;
                    }
                }
                if retired == before {
                    break;
                }
            }
            tracing::info!(
                audit_retained_blocks_deleted = retired,
                "audit retention pass"
            );
            Ok::<(), SealError>(())
        };
        match tokio::time::timeout(Duration::from_secs(2), work).await {
            Ok(result) => result,
            Err(_) => {
                tracing::warn!("audit retention pass reached its two-second work budget");
                Ok(())
            }
        }
    }
}
