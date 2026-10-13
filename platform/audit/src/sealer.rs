//! The installation's one sealer. It holds the `audit.sealer` singleton role, reads
//! records in commit order below the snapshot horizon, signs one block per partition and
//! commits the blocks with the sealed position on the connection that holds the role.
//! Export receipts and retention also write through that connection.
use crate::{
    integrity::{AuditSigningKey, IntegrityError},
    service::{AuditHealthState, HealthUpdate, Maintenance},
    store::{AuditSignal, AuditStore, AuditStoreError, delivery::ExportTransition},
};
use chrono::{DateTime, Utc};
use futures::StreamExt;
use sqlx::Connection;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::{Mutex, watch};
use veoveo_audit_contract::*;
use veoveo_platform_store::{Singleton, SingletonName, StoreError};

/// The most records one pass seals.
const SEAL_PAGE: usize = 4096;
/// After the first record of a quiet period commits, the sealer waits this long so
/// records committed together share a block.
const BATCH_WINDOW: Duration = Duration::from_secs(1);
/// How soon the sealer looks again at records that wait behind an open transaction.
const BLOCKED_RETRY: Duration = Duration::from_millis(250);
/// How often the sealer checks the connection that holds its role.
const HEARTBEAT: Duration = Duration::from_secs(10);
/// How often retention runs.
const MAINTENANCE: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum SealError {
    #[error(transparent)]
    Store(#[from] AuditStoreError),
    #[error(transparent)]
    Integrity(#[from] IntegrityError),
    #[error(transparent)]
    Export(#[from] crate::export::ExportError),
    #[error("the audit exporter stopped unexpectedly")]
    Exporter,
}

impl From<StoreError> for SealError {
    fn from(error: StoreError) -> Self {
        Self::Store(error.into())
    }
}

impl From<sqlx::Error> for SealError {
    fn from(error: sqlx::Error) -> Self {
        Self::Store(error.into())
    }
}

/// Committed records that remain after a seal pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealBacklog {
    /// Every committed record is sealed.
    Empty,
    /// The pass was full and more records can be sealed now.
    Sealable,
    /// Committed records wait behind a write transaction that is still open. Every
    /// record whose transaction began writing earlier must commit or roll back first.
    Blocked,
}

/// What one seal pass did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SealPass {
    pub sealed: usize,
    pub blocks: usize,
    pub backlog: SealBacklog,
}

/// The holder of the sealer role. Creating one waits, as a standby, until no other
/// replica holds the role; dropping or releasing it ends the role's connection and
/// Postgres grants the role to the next waiting replica.
pub struct AuditSealer {
    audit: AuditStore,
    key: Arc<AuditSigningKey>,
    role: Mutex<Singleton>,
}

impl std::fmt::Debug for AuditSealer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditSealer")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl AuditSealer {
    /// The singleton role every sealer contends for.
    pub const ROLE: &str = "audit.sealer";

    /// Wait until this process holds the sealer role. The wait has no deadline.
    pub async fn acquire(audit: AuditStore, key: Arc<AuditSigningKey>) -> Result<Self, SealError> {
        let name = SingletonName::parse(Self::ROLE)?;
        let role = Singleton::acquire(audit.platform(), name).await?;
        Ok(Self {
            audit,
            key,
            role: Mutex::new(role),
        })
    }

    pub fn audit(&self) -> &AuditStore {
        &self.audit
    }

    /// Give up the role.
    pub async fn release(self) -> Result<(), SealError> {
        self.role.into_inner().release().await?;
        Ok(())
    }

    /// Seal up to 4,096 committed records: one block for each partition they belong to.
    pub async fn seal_pending(&self) -> Result<SealPass, SealError> {
        let mut role = self.role.lock().await;
        let connection = role.connection();
        let from = AuditStore::sealed_position(connection).await?;
        let mut candidates = AuditStore::seal_candidates(connection, SEAL_PAGE + 1).await?;
        let full = candidates.len() > SEAL_PAGE;
        candidates.truncate(SEAL_PAGE);
        let Some(to) = candidates.last().map(|candidate| candidate.position) else {
            let backlog = if AuditStore::seal_pending(connection).await? {
                SealBacklog::Blocked
            } else {
                SealBacklog::Empty
            };
            return Ok(SealPass {
                sealed: 0,
                blocks: 0,
                backlog,
            });
        };
        let mut previous = from;
        let mut partitions = BTreeMap::<AuditPartition, Vec<AuditStoredRecord>>::new();
        for candidate in candidates {
            if candidate.position <= previous {
                return Err(AuditStoreError::Integrity.into());
            }
            previous = candidate.position;
            partitions
                .entry(candidate.record.partition().clone())
                .or_default()
                .push(candidate.record);
        }
        let sealed = partitions.values().map(Vec::len).sum();
        let sealed_at = stored_time(Utc::now());
        let mut blocks = Vec::with_capacity(partitions.len());
        for (partition, records) in partitions {
            let head = AuditStore::head(connection, &partition).await?;
            blocks.push(
                self.key
                    .seal(partition, head.as_ref(), &records, sealed_at)?,
            );
        }
        AuditStore::commit_seal(connection, from, to, &blocks).await?;
        tracing::info!(
            audit_sealed_records = sealed,
            audit_sealed_blocks = blocks.len(),
            "audit seal committed"
        );
        let backlog = if full {
            SealBacklog::Sealable
        } else if AuditStore::seal_pending(connection).await? {
            SealBacklog::Blocked
        } else {
            SealBacklog::Empty
        };
        Ok(SealPass {
            sealed,
            blocks: blocks.len(),
            backlog,
        })
    }

    /// Commit the intent to send `block` to `destination`, with the hashes of the bytes
    /// the exporter will send, before sending anything.
    pub async fn prepare_export(
        &self,
        destination: &AuditDestinationId,
        block: &AuditBlock,
        payload: &AuditExportPayload,
    ) -> Result<(), AuditStoreError> {
        self.record_export(destination, block, payload, ExportTransition::Prepare, None)
            .await
    }

    /// Record the destination's acknowledgement and advance its cursor.
    pub async fn complete_export(
        &self,
        destination: &AuditDestinationId,
        block: &AuditBlock,
        payload: &AuditExportPayload,
    ) -> Result<(), AuditStoreError> {
        self.record_export(
            destination,
            block,
            payload,
            ExportTransition::Complete,
            None,
        )
        .await
    }

    /// Record the destination's permanent refusal. No replica retries the intent.
    pub async fn reject_export(
        &self,
        destination: &AuditDestinationId,
        block: &AuditBlock,
        payload: &AuditExportPayload,
        rejection: AuditExportRejection,
    ) -> Result<(), AuditStoreError> {
        self.record_export(
            destination,
            block,
            payload,
            ExportTransition::Reject,
            Some(rejection),
        )
        .await
    }

    async fn record_export(
        &self,
        destination: &AuditDestinationId,
        block: &AuditBlock,
        payload: &AuditExportPayload,
        transition: ExportTransition,
        rejection: Option<AuditExportRejection>,
    ) -> Result<(), AuditStoreError> {
        let mut role = self.role.lock().await;
        AuditStore::record_export(
            role.connection(),
            destination,
            block,
            payload,
            transition,
            rejection,
        )
        .await
    }

    /// Retire `block` and its records when it was sealed before `cutoff` and every
    /// destination acknowledged it. Returns false when retention does not admit it.
    pub async fn retire(
        &self,
        block: &AuditBlock,
        cutoff: DateTime<Utc>,
        destinations: &[AuditDestinationId],
    ) -> Result<bool, AuditStoreError> {
        let mut role = self.role.lock().await;
        AuditStore::retire_block(role.connection(), block, cutoff, destinations).await
    }

    async fn heartbeat(&self) -> Result<(), SealError> {
        let mut role = self.role.lock().await;
        role.connection().ping().await?;
        Ok(())
    }

    /// Seal every committed record. Producers have stopped; a record that waits behind
    /// an open transaction is sealed once that transaction ends.
    async fn drain(&self) -> Result<(), SealError> {
        loop {
            match self.seal_pending().await?.backlog {
                SealBacklog::Empty => return Ok(()),
                SealBacklog::Sealable => {}
                SealBacklog::Blocked => tokio::time::sleep(BLOCKED_RETRY).await,
            }
        }
    }

    /// Seal records as they commit, run export and retention, and drain on shutdown. A
    /// sealer that loses its connection returns, so the supervisor can wait for the
    /// role again.
    pub(crate) async fn run(
        self: Arc<Self>,
        mut shutdown: watch::Receiver<bool>,
        health: &watch::Sender<HealthUpdate>,
        maintenance: &Maintenance,
    ) -> Result<(), SealError> {
        health.send_replace(HealthUpdate::now(AuditHealthState::Recovering));
        let mut changes = self.audit.subscribe().await?;
        let mut export = tokio::spawn(maintenance.exporter.clone().run(self.clone()));
        let result = self
            .serve(
                &mut shutdown,
                health,
                maintenance,
                &mut changes,
                &mut export,
            )
            .await;
        export.abort();
        let _ = export.await;
        result
    }

    async fn serve(
        &self,
        shutdown: &mut watch::Receiver<bool>,
        health: &watch::Sender<HealthUpdate>,
        maintenance: &Maintenance,
        changes: &mut veoveo_platform_store::ChangeStream,
        export: &mut tokio::task::JoinHandle<Result<(), crate::export::ExportError>>,
    ) -> Result<(), SealError> {
        let mut heartbeat = tokio::time::interval(HEARTBEAT);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut maintain = tokio::time::interval(MAINTENANCE);
        maintain.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Recovery seals whatever committed while no replica held the role.
        let mut pending = true;
        let mut flush_at = tokio::time::Instant::now();
        let mut caught_up = false;
        loop {
            if *shutdown.borrow() {
                return self.drain().await;
            }
            tokio::select! {
                result = &mut *export => {
                    return Err(match result {
                        Ok(Err(error)) => error.into(),
                        Ok(Ok(())) | Err(_) => SealError::Exporter,
                    });
                }
                result = shutdown.changed() => {
                    if result.is_err() || *shutdown.borrow() {
                        return self.drain().await;
                    }
                }
                signal = changes.next() => match signal.map(|signal| signal.map(AuditSignal::decode)) {
                    Some(Ok(AuditSignal::Record(_) | AuditSignal::Reconcile)) => {
                        if !pending {
                            pending = true;
                            flush_at = tokio::time::Instant::now() + BATCH_WINDOW;
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        tracing::warn!(%error, "audit sealer listener reconnecting");
                    }
                    None => return Err(AuditStoreError::Disconnected.into()),
                },
                _ = heartbeat.tick() => {
                    self.heartbeat().await?;
                    if caught_up {
                        health.send_replace(HealthUpdate::now(AuditHealthState::Active));
                    }
                }
                _ = maintain.tick() => maintenance.run(self).await?,
                _ = tokio::time::sleep_until(flush_at), if pending => {
                    let pass = self.seal_pending().await?;
                    match pass.backlog {
                        SealBacklog::Sealable => flush_at = tokio::time::Instant::now(),
                        SealBacklog::Blocked => {
                            flush_at = tokio::time::Instant::now() + BLOCKED_RETRY;
                        }
                        SealBacklog::Empty => {
                            pending = false;
                            caught_up = true;
                            health.send_replace(HealthUpdate::now(AuditHealthState::Active));
                        }
                    }
                    // Return to select between passes so shutdown, export and the
                    // heartbeat progress while a large backlog drains.
                    tokio::task::yield_now().await;
                }
            }
        }
    }
}
