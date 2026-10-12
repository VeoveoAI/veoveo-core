//! Change streams for one partition. Notifications are wake signals: each stream rereads
//! the rows they name under the reader's scope and drops the ones of other partitions.
use super::{AuditSignal, AuditStore, AuditStoreError, admit};
use futures::{
    StreamExt,
    stream::{BoxStream, ReadyChunks},
};
use std::collections::VecDeque;
use veoveo_audit_contract::{AuditPartition, AuditReadScope, AuditRecord, AuditRecordId};
use veoveo_platform_store::{ChangeSignal, ChangeStream, StoreError};

/// What a partition's record stream reports.
#[derive(Debug)]
pub enum AuditLiveChange {
    /// A record of the partition committed.
    Record(Box<AuditRecord>),
    /// Retention removed one of the partition's blocks and its records.
    RetainedRangeChanged,
    /// The listener reconnected and may have missed changes; reread the partition.
    Reconcile,
}

pub type AuditLiveStream = BoxStream<'static, Result<AuditLiveChange, AuditStoreError>>;

/// Signals that arrive together are read with one query.
const SIGNAL_BATCH: usize = 256;

struct Live {
    audit: AuditStore,
    partition: AuditPartition,
    changes: ReadyChunks<ChangeStream>,
    pending: VecDeque<Result<AuditLiveChange, AuditStoreError>>,
}

impl AuditStore {
    /// Committed records of one partition, starting with the first record that commits
    /// after this call returns.
    pub async fn live(
        &self,
        scope: &AuditReadScope,
        partition: &AuditPartition,
    ) -> Result<AuditLiveStream, AuditStoreError> {
        admit(scope, partition)?;
        let live = Live {
            audit: self.clone(),
            partition: partition.clone(),
            changes: self.subscribe().await?.ready_chunks(SIGNAL_BATCH),
            pending: VecDeque::new(),
        };
        Ok(futures::stream::unfold(live, |mut live| async move {
            loop {
                if let Some(item) = live.pending.pop_front() {
                    return Some((item, live));
                }
                let signals = live.changes.next().await?;
                live.read(signals).await;
            }
        })
        .boxed())
    }

    /// One item each time a block of the partition is sealed, and after the listener
    /// reconnects. Readers replay the partition's blocks from their own checkpoint.
    pub async fn blocks_live(
        &self,
        scope: &AuditReadScope,
        partition: &AuditPartition,
    ) -> Result<BoxStream<'static, Result<(), AuditStoreError>>, AuditStoreError> {
        admit(scope, partition)?;
        let key = partition.storage_key();
        let changes = self.subscribe().await?;
        Ok(changes
            .filter_map(move |signal| {
                let item = match signal.map(AuditSignal::decode) {
                    Ok(AuditSignal::Reconcile) => Some(Ok(())),
                    Ok(AuditSignal::Sealed(sealed)) if sealed == key => Some(Ok(())),
                    Ok(_) => None,
                    Err(error) => Some(Err(error.into())),
                };
                std::future::ready(item)
            })
            .boxed())
    }

    /// Return once the record is sealed. The caller bounds the wait; a record that never
    /// commits is never sealed.
    pub async fn wait_sealed(
        &self,
        scope: &AuditReadScope,
        partition: &AuditPartition,
        record: AuditRecordId,
    ) -> Result<(), AuditStoreError> {
        admit(scope, partition)?;
        let key = partition.storage_key();
        let mut changes = self.subscribe().await?;
        loop {
            let sealed = sqlx::query_file_scalar!(
                "src/store/live/sealed.sql",
                record as AuditRecordId,
                &key,
            )
            .fetch_one(self.pool())
            .await?;
            if sealed {
                return Ok(());
            }
            loop {
                match changes
                    .next()
                    .await
                    .map(|signal| signal.map(AuditSignal::decode))
                {
                    Some(Ok(AuditSignal::Reconcile)) => break,
                    Some(Ok(AuditSignal::Sealed(sealed))) if sealed == key => break,
                    // The listener reconnects by itself and then asks to reconcile.
                    Some(Ok(_) | Err(_)) => {}
                    None => return Err(AuditStoreError::Disconnected),
                }
            }
        }
    }
}

impl Live {
    async fn read(&mut self, signals: Vec<Result<ChangeSignal, StoreError>>) {
        let key = self.partition.storage_key();
        let mut records = Vec::new();
        let mut reconcile = false;
        let mut retired = false;
        for signal in signals {
            match signal.map(AuditSignal::decode) {
                Ok(AuditSignal::Reconcile) => reconcile = true,
                Ok(AuditSignal::Record(id)) => records.extend(AuditRecordId::try_from(id).ok()),
                Ok(AuditSignal::Retired(partition)) if partition == key => retired = true,
                Ok(_) => {}
                Err(error) => self.pending.push_back(Err(error.into())),
            }
        }
        if reconcile {
            self.pending.push_back(Ok(AuditLiveChange::Reconcile));
        }
        if !records.is_empty() {
            match self.audit.records_by_id(&self.partition, &records).await {
                Ok(mut found) => self.pending.extend(
                    records
                        .iter()
                        .filter_map(|id| found.remove(id))
                        .map(|record| Ok(AuditLiveChange::Record(Box::new(record)))),
                ),
                Err(error) => self.pending.push_back(Err(error)),
            }
        }
        if retired {
            self.pending
                .push_back(Ok(AuditLiveChange::RetainedRangeChanged));
        }
    }
}
