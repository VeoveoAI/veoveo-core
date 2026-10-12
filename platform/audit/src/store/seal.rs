//! The sealer's reads and writes. They run on the connection that holds the sealer
//! role, so a sealer that lost the role cannot write.
use super::{
    AuditStore, AuditStoreError,
    blocks::checkpoint,
    row::{RecordRow, stored_sequence},
};
use chrono::{DateTime, Utc};
use sqlx::{Connection, PgConnection};
use veoveo_audit_contract::{
    AuditBlock, AuditCheckpoint, AuditClass, AuditPartition, AuditRecord, AuditRecordId,
    stored_time,
};

/// A commit position: the writing transaction and the insertion sequence of a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CommitPosition {
    pub txid: i64,
    pub sequence: i64,
}

/// A record the sealer may seal, with its commit position.
#[derive(Debug)]
pub(crate) struct SealCandidate {
    pub position: CommitPosition,
    pub record: AuditRecord,
}

struct CandidateRow {
    sequence: i64,
    txid: i64,
    id: AuditRecordId,
    partition: String,
    class: AuditClass,
    occurred_at: DateTime<Utc>,
    draft: serde_json::Value,
    recorded_at: DateTime<Utc>,
}

impl AuditStore {
    pub(crate) async fn sealed_position(
        connection: &mut PgConnection,
    ) -> Result<CommitPosition, AuditStoreError> {
        let row = sqlx::query_file!("src/store/seal/cursor.sql")
            .fetch_one(connection)
            .await?;
        Ok(CommitPosition {
            txid: row.txid,
            sequence: row.sequence,
        })
    }

    /// Up to `limit` sealable records in commit order.
    pub(crate) async fn seal_candidates(
        &self,
        connection: &mut PgConnection,
        limit: usize,
    ) -> Result<Vec<SealCandidate>, AuditStoreError> {
        let limit = i64::try_from(limit).map_err(|_| AuditStoreError::BatchLimit)?;
        let rows = sqlx::query_file_as!(CandidateRow, "src/store/seal/candidates.sql", limit)
            .fetch_all(connection)
            .await?;
        rows.into_iter()
            .map(|row| {
                let record = RecordRow {
                    id: row.id,
                    partition: row.partition,
                    class: row.class,
                    occurred_at: row.occurred_at,
                    draft: row.draft,
                    recorded_at: row.recorded_at,
                }
                .admitted(self.targets())?;
                Ok(SealCandidate {
                    position: CommitPosition {
                        txid: row.txid,
                        sequence: row.sequence,
                    },
                    record,
                })
            })
            .collect()
    }

    /// Whether a committed record follows the sealed position, sealable or not.
    pub(crate) async fn seal_pending(
        connection: &mut PgConnection,
    ) -> Result<bool, AuditStoreError> {
        Ok(sqlx::query_file_scalar!("src/store/seal/pending.sql")
            .fetch_one(connection)
            .await?)
    }

    pub(crate) async fn head(
        connection: &mut PgConnection,
        partition: &AuditPartition,
    ) -> Result<Option<AuditCheckpoint>, AuditStoreError> {
        let row = sqlx::query_file!("src/store/blocks/checkpoint.sql", partition.storage_key())
            .fetch_optional(connection)
            .await?;
        row.map(|row| checkpoint(partition, row.sequence, row.last_position, row.head_hash))
            .transpose()
    }

    /// Store the blocks, advance each partition's head and move the sealed position from
    /// `from` to `to` in one transaction. A head or position that changed meanwhile
    /// fails the commit.
    pub(crate) async fn commit_seal(
        connection: &mut PgConnection,
        from: CommitPosition,
        to: CommitPosition,
        blocks: &[AuditBlock],
    ) -> Result<(), AuditStoreError> {
        if to < from {
            return Err(AuditStoreError::Integrity);
        }
        let mut transaction = connection.begin().await?;
        for block in blocks {
            let head = &block.head;
            let partition = head.partition.storage_key();
            let sequence = stored_sequence(head.sequence);
            sqlx::query_file!(
                "src/store/seal/insert_block.sql",
                &partition,
                sequence,
                block.head_hash.as_str(),
                stored_time(head.sealed_at),
                serde_json::to_value(block).expect("a signed block serializes"),
            )
            .execute(&mut *transaction)
            .await?;
            let advanced = sqlx::query_file!(
                "src/store/seal/advance_head.sql",
                &partition,
                sequence,
                i64::try_from(head.last_position.get()).map_err(|_| AuditStoreError::Integrity)?,
                block.head_hash.as_str(),
                head.previous.as_ref().map(|previous| previous.as_str()),
            )
            .execute(&mut *transaction)
            .await?
            .rows_affected();
            if advanced != 1 {
                return Err(AuditStoreError::Integrity);
            }
        }
        let moved = sqlx::query_file!(
            "src/store/seal/advance_sealer.sql",
            to.txid,
            to.sequence,
            from.txid,
            from.sequence,
        )
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if moved != 1 {
            return Err(AuditStoreError::Integrity);
        }
        transaction.commit().await?;
        Ok(())
    }
}
