//! Partition-scoped reads of signed blocks, their member records and the retained range.
use super::{
    AuditStore, AuditStoreError, admit,
    records::target_document,
    row::{BlockRow, RecordRow, sequence, stored_sequence},
};
use veoveo_audit_contract::{
    AUDIT_APPEND_LIMIT, AuditBlock, AuditBlockSequence, AuditCheckpoint, AuditClass, AuditOutcome,
    AuditPartition, AuditPosition, AuditQuery, AuditReadScope, AuditRecord, AuditRecordId,
    AuditTraceId, AuditValidationError,
};
use veoveo_types::{PrincipalId, Sha256Digest};

/// The retained interval of a partition, fixed before an export starts. A later
/// retention pass makes the export fail rather than silently move its first block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditExportRange {
    pub checkpoint: Option<AuditCheckpoint>,
    pub first: Option<AuditBlockSequence>,
}

impl AuditStore {
    /// Up to `limit` blocks of a partition after `after`, in chain order.
    pub async fn blocks(
        &self,
        scope: &AuditReadScope,
        partition: &AuditPartition,
        after: Option<AuditBlockSequence>,
        limit: u16,
    ) -> Result<Vec<AuditBlock>, AuditStoreError> {
        admit(scope, partition)?;
        if limit == 0 || limit > 100 {
            return Err(AuditStoreError::BatchLimit);
        }
        let rows = sqlx::query_file_as!(
            BlockRow,
            "src/store/blocks/blocks.sql",
            partition.storage_key(),
            after.map_or(0, stored_sequence),
            i64::from(limit),
        )
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(BlockRow::checked).collect()
    }

    /// The member records of a block in position order. A missing member fails the
    /// read.
    pub async fn block_records(
        &self,
        scope: &AuditReadScope,
        block: &AuditBlock,
    ) -> Result<Vec<AuditRecord>, AuditStoreError> {
        let partition = &block.head.partition;
        admit(scope, partition)?;
        let ids = member_ids(block)?;
        let mut records = self.records_by_id(partition, &ids).await?;
        ids.iter()
            .map(|id| records.remove(id).ok_or(AuditStoreError::Integrity))
            .collect()
    }

    /// Every member of a block that matches `query`'s filters, in position order, as an
    /// export reads it. The query's order and page size select pages and do not apply
    /// here, and a query with a cursor is refused, because a block holds no pages. Every
    /// filter runs in SQL; a missing member fails the read even when the filter would
    /// exclude it.
    pub async fn filtered_block_records(
        &self,
        scope: &AuditReadScope,
        block: &AuditBlock,
        query: &AuditQuery,
    ) -> Result<Vec<AuditRecord>, AuditStoreError> {
        query.validate()?;
        if query.cursor.is_some() {
            return Err(AuditValidationError::Cursor.into());
        }
        if let Some(target) = &query.target {
            self.targets().validate(target)?;
        }
        let partition = &block.head.partition;
        admit(scope, partition)?;
        if &query.partition != partition {
            return Err(AuditStoreError::AccessDenied);
        }
        let ids = member_ids(block)?;
        let storage = partition.storage_key();
        let mut transaction = self.pool().begin().await?;
        sqlx::query_file!("src/store/blocks/snapshot.sql")
            .execute(&mut *transaction)
            .await?;
        let present = sqlx::query_file_scalar!(
            "src/store/blocks/members_present.sql",
            &storage,
            &ids as &[AuditRecordId],
        )
        .fetch_one(&mut *transaction)
        .await?;
        if usize::try_from(present).ok() != Some(ids.len()) {
            return Err(AuditStoreError::Integrity);
        }
        let rows = sqlx::query_file_as!(
            RecordRow,
            "src/store/blocks/filtered_members.sql",
            &storage,
            &ids as &[AuditRecordId],
            query.class as Option<AuditClass>,
            query.actor.as_ref().map(PrincipalId::as_str),
            query.target.as_ref().map(target_document),
            query.outcome as Option<AuditOutcome>,
            query.trace.as_ref() as Option<&AuditTraceId>,
            query.from,
            query.until,
        )
        .fetch_all(&mut *transaction)
        .await?;
        transaction.commit().await?;
        let mut records = rows
            .into_iter()
            .map(|row| {
                let record = row.checked(self.targets(), partition)?;
                Ok((record.draft.id(), record))
            })
            .collect::<Result<std::collections::BTreeMap<_, _>, AuditStoreError>>()?;
        Ok(ids.iter().filter_map(|id| records.remove(id)).collect())
    }

    /// The partition's newest signed head, which retention keeps.
    pub async fn partition_checkpoint(
        &self,
        partition: &AuditPartition,
    ) -> Result<Option<AuditCheckpoint>, AuditStoreError> {
        let row = sqlx::query_file!("src/store/blocks/checkpoint.sql", partition.storage_key())
            .fetch_optional(self.pool())
            .await?;
        row.map(|row| checkpoint(partition, row.sequence, row.last_position, row.head_hash))
            .transpose()
    }

    /// The partition's retained blocks: the first one not yet retired and the newest
    /// signed head.
    pub async fn export_range(
        &self,
        scope: &AuditReadScope,
        partition: &AuditPartition,
    ) -> Result<AuditExportRange, AuditStoreError> {
        admit(scope, partition)?;
        let row = sqlx::query_file!("src/store/blocks/export_range.sql", partition.storage_key())
            .fetch_one(self.pool())
            .await?;
        let head = match (row.sequence, row.last_position, row.head_hash) {
            (Some(sequence), Some(last_position), Some(head_hash)) => {
                Some(checkpoint(partition, sequence, last_position, head_hash)?)
            }
            (None, None, None) => None,
            _ => return Err(AuditStoreError::Integrity),
        };
        let retired = row
            .retired
            .map_or(Ok(0), u64::try_from)
            .map_err(|_| AuditStoreError::Integrity)?;
        let first = match &head {
            None if retired == 0 => None,
            None => return Err(AuditStoreError::Integrity),
            Some(head) if retired > head.sequence.get() => {
                return Err(AuditStoreError::Integrity);
            }
            Some(head) if retired == head.sequence.get() => None,
            Some(_) => Some(sequence(
                i64::try_from(retired + 1).map_err(|_| AuditStoreError::Integrity)?,
            )?),
        };
        Ok(AuditExportRange {
            checkpoint: head,
            first,
        })
    }
}

pub(crate) fn checkpoint(
    partition: &AuditPartition,
    block: i64,
    last_position: i64,
    head_hash: String,
) -> Result<AuditCheckpoint, AuditStoreError> {
    Ok(AuditCheckpoint {
        partition: partition.clone(),
        sequence: sequence(block)?,
        head_hash: Sha256Digest::parse(head_hash).map_err(|_| AuditStoreError::Integrity)?,
        last_position: u64::try_from(last_position)
            .ok()
            .and_then(|value| AuditPosition::new(value).ok())
            .ok_or(AuditStoreError::Integrity)?,
    })
}

fn member_ids(block: &AuditBlock) -> Result<Vec<AuditRecordId>, AuditStoreError> {
    if block.head.members.len() > AUDIT_APPEND_LIMIT {
        return Err(AuditStoreError::BatchLimit);
    }
    Ok(block.head.members.iter().map(|member| member.id).collect())
}
