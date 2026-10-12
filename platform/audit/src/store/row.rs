//! Stored records and blocks decode through the target registry and are checked
//! against the columns that selected them.
use super::AuditStoreError;
use chrono::{DateTime, Utc};
use veoveo_audit_contract::{
    AuditBlock, AuditBlockSequence, AuditClass, AuditDecodeError, AuditDraft, AuditPartition,
    AuditRecord, AuditRecordId, AuditTargetRegistry, stored_time,
};

/// One `audit.record` row as readers select it.
#[derive(Debug)]
pub(crate) struct RecordRow {
    pub id: AuditRecordId,
    pub partition: String,
    pub class: AuditClass,
    pub occurred_at: DateTime<Utc>,
    pub draft: serde_json::Value,
    pub recorded_at: DateTime<Utc>,
}

impl RecordRow {
    /// Admit the stored draft and require it to agree with its row.
    pub fn admitted(self, registry: &AuditTargetRegistry) -> Result<AuditRecord, AuditStoreError> {
        let draft: AuditDraft =
            registry
                .decoder()
                .from_value(self.draft)
                .map_err(|error| match error {
                    AuditDecodeError::Target(error) => AuditStoreError::Target(error),
                    _ => AuditStoreError::Integrity,
                })?;
        if draft.id() != self.id
            || self.partition != draft.partition().storage_key()
            || draft.detail().class() != self.class
            || stored_time(draft.occurred_at()) != self.occurred_at
        {
            return Err(AuditStoreError::Integrity);
        }
        Ok(AuditRecord {
            draft,
            recorded_at: self.recorded_at,
        })
    }

    /// Admit the stored draft and require it to belong to the partition the query
    /// selected.
    pub fn checked(
        self,
        registry: &AuditTargetRegistry,
        partition: &AuditPartition,
    ) -> Result<AuditRecord, AuditStoreError> {
        let record = self.admitted(registry)?;
        if record.draft.partition() != partition {
            return Err(AuditStoreError::Integrity);
        }
        Ok(record)
    }
}

/// One `audit.block` row.
#[derive(Debug)]
pub(crate) struct BlockRow {
    pub partition: String,
    pub sequence: i64,
    pub head_hash: String,
    pub sealed_at: DateTime<Utc>,
    pub block: serde_json::Value,
}

impl BlockRow {
    pub fn checked(self) -> Result<AuditBlock, AuditStoreError> {
        let block: AuditBlock =
            serde_json::from_value(self.block).map_err(|_| AuditStoreError::Integrity)?;
        if self.partition != block.head.partition.storage_key()
            || sequence(self.sequence)? != block.head.sequence
            || self.head_hash != block.head_hash.as_str()
            || self.sealed_at != stored_time(block.head.sealed_at)
        {
            return Err(AuditStoreError::Integrity);
        }
        Ok(block)
    }
}

pub(crate) fn sequence(value: i64) -> Result<AuditBlockSequence, AuditStoreError> {
    u64::try_from(value)
        .ok()
        .and_then(|value| AuditBlockSequence::new(value).ok())
        .ok_or(AuditStoreError::Integrity)
}

pub(crate) fn stored_sequence(value: AuditBlockSequence) -> i64 {
    i64::try_from(value.get()).expect("admitted block sequences fit i64")
}
