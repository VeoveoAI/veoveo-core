//! Export delivery and retention state. Candidates are read from the pool; intents,
//! receipts, refusals and retention run on the sealer's connection.
use super::{
    AuditStore, AuditStoreError,
    row::{BlockRow, stored_sequence},
};
use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use veoveo_audit_contract::{
    AuditBlock, AuditDestinationId, AuditExportPayload, AuditExportRejection, AuditRecordId,
};

/// The step of an export that `audit.record_export` records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, veoveo_types::Vocabulary)]
#[vocabulary(postgres)]
pub(crate) enum ExportTransition {
    /// Commit the intent and its payload hashes before anything is sent.
    Prepare,
    /// Record the destination's acknowledgement.
    Complete,
    /// Record the destination's permanent refusal.
    Reject,
}

impl AuditStore {
    /// The next block `destination` has not acknowledged, from the partition it served
    /// least recently.
    pub async fn export_candidate(
        &self,
        destination: &AuditDestinationId,
    ) -> Result<Option<AuditBlock>, AuditStoreError> {
        let row = sqlx::query_file!("src/store/delivery/candidate.sql", destination.as_str())
            .fetch_optional(self.pool())
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        match (
            row.partition,
            row.sequence,
            row.head_hash,
            row.sealed_at,
            row.block,
        ) {
            (Some(partition), Some(sequence), Some(head_hash), Some(sealed_at), Some(block)) => {
                BlockRow {
                    partition,
                    sequence,
                    head_hash,
                    sealed_at,
                    block,
                }
                .checked()
                .map(Some)
            }
            // A head names a block that is neither retained nor retired.
            _ => Err(AuditStoreError::Integrity),
        }
    }

    pub(crate) async fn record_export(
        connection: &mut PgConnection,
        destination: &AuditDestinationId,
        block: &AuditBlock,
        payload: &AuditExportPayload,
        transition: ExportTransition,
        rejection: Option<AuditExportRejection>,
    ) -> Result<(), AuditStoreError> {
        sqlx::query_file!(
            "src/store/delivery/record_export.sql",
            block.head.partition.storage_key(),
            stored_sequence(block.head.sequence),
            block.head_hash.as_str(),
            destination.as_str(),
            payload.content.as_str(),
            payload.seal.as_str(),
            transition as ExportTransition,
            rejection as Option<AuditExportRejection>,
        )
        .execute(connection)
        .await?;
        Ok(())
    }

    /// Each partition's oldest retained block that retention admits at `cutoff`.
    pub(crate) async fn retention_candidates(
        &self,
        cutoff: DateTime<Utc>,
        destinations: &[AuditDestinationId],
    ) -> Result<Vec<AuditBlock>, AuditStoreError> {
        let destinations = destinations
            .iter()
            .map(|destination| destination.as_str().to_owned())
            .collect::<Vec<_>>();
        let rows = sqlx::query_file_as!(
            BlockRow,
            "src/store/delivery/retention_candidates.sql",
            cutoff,
            &destinations,
        )
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(BlockRow::checked).collect()
    }

    /// Retire the block and its records, keeping it as the partition's anchor. Returns
    /// false when retention no longer admits the block.
    pub(crate) async fn retire_block(
        connection: &mut PgConnection,
        block: &AuditBlock,
        cutoff: DateTime<Utc>,
        destinations: &[AuditDestinationId],
    ) -> Result<bool, AuditStoreError> {
        let destinations = destinations
            .iter()
            .map(|destination| destination.as_str().to_owned())
            .collect::<Vec<_>>();
        let records = block
            .head
            .members
            .iter()
            .map(|member| member.id)
            .collect::<Vec<_>>();
        Ok(sqlx::query_file_scalar!(
            "src/store/delivery/retire.sql",
            block.head.partition.storage_key(),
            stored_sequence(block.head.sequence),
            block.head_hash.as_str(),
            cutoff,
            &destinations,
            &records as &[AuditRecordId],
        )
        .fetch_one(connection)
        .await?)
    }
}
