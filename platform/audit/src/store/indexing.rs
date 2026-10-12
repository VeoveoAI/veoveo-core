//! Admitted indexing reads accumulate into database-clock windows with retry receipts.
//! Any replica finalizes an ended window into one immutable record.
use super::{AuditStore, AuditStoreError};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use std::time::Duration;
use veoveo_audit_contract::{
    AuditDecodeError, AuditDetail, AuditDraft, AuditOutcome, AuditReadMethod, AuditReason,
    AuditRecordId, AuditRequest, AuditTarget, AuditTargetRegistry, AuditTransactionWrite,
    IndexingRead,
};
use veoveo_mcp_ext_knowledge::CollectionId;
use veoveo_types::Sha256Digest;

fn digest(value: impl serde::Serialize) -> String {
    let bytes = serde_json::to_vec(&value).expect("a typed audit value serializes");
    Sha256Digest::from_bytes(Sha256::digest(bytes).into()).to_string()
}

/// Admitted reads as the parallel arrays `audit.append_indexing` takes.
#[derive(Debug, Default)]
pub(crate) struct IndexingRows {
    partitions: Vec<String>,
    records: Vec<AuditRecordId>,
    fingerprints: Vec<String>,
    scopes: Vec<String>,
    occurred: Vec<DateTime<Utc>>,
    collections: Vec<String>,
    templates: Vec<serde_json::Value>,
    members: Vec<Option<String>>,
    not_modified: Vec<i64>,
    failed: Vec<i64>,
}

impl IndexingRows {
    /// A read's window is keyed by its actor, authority and collection. Its fingerprint
    /// binds the draft to the collection, so a retry with other content fails.
    pub fn encode(
        registry: &AuditTargetRegistry,
        reads: &[IndexingRead],
    ) -> Result<Self, AuditStoreError> {
        let mut rows = Self::default();
        for read in reads {
            let draft = read.draft();
            registry.validate(draft.target())?;
            let actor = draft.actor().ok_or(AuditStoreError::Integrity)?;
            let template = AuditDraft::builder(
                AuditRequest::background(),
                AuditTarget::Server {
                    server: read.collection().server().clone(),
                },
                AuditDetail::Read {
                    method: AuditReadMethod::ResourceRead,
                },
                AuditOutcome::Succeeded,
                AuditReason::Accepted,
            )
            .actor(actor.clone())
            .authority(draft.authority().clone())
            .build()?;
            let observation = match draft.detail() {
                AuditDetail::KnowledgeRead { observation, .. } => observation.as_deref(),
                _ => None,
            };
            let member = match (draft.target(), observation) {
                (AuditTarget::Resource { uri, .. }, Some(observation)) => {
                    Some(digest((uri, &observation.revision)))
                }
                _ => None,
            };
            rows.partitions.push(draft.partition().storage_key());
            rows.records.push(draft.id());
            rows.fingerprints.push(digest((read.collection(), draft)));
            rows.scopes
                .push(digest((actor, draft.authority(), read.collection())));
            rows.occurred.push(draft.occurred_at());
            rows.collections.push(read.collection().to_string());
            rows.templates
                .push(serde_json::to_value(&template).expect("an admitted template serializes"));
            rows.members.push(member);
            rows.not_modified
                .push(i64::from(observation.is_some_and(|o| o.not_modified)));
            rows.failed
                .push(i64::from(draft.outcome() == AuditOutcome::Failed));
        }
        Ok(rows)
    }

    pub async fn append(&self, connection: &mut PgConnection) -> Result<(), AuditStoreError> {
        if self.records.is_empty() {
            return Ok(());
        }
        sqlx::query_file!(
            "src/store/indexing/append.sql",
            &self.partitions,
            &self.records as &[AuditRecordId],
            &self.fingerprints,
            &self.scopes,
            &self.occurred,
            &self.collections,
            &self.templates,
            &self.members as &[Option<String>],
            &self.not_modified,
            &self.failed,
        )
        .execute(connection)
        .await?;
        Ok(())
    }
}

impl AuditStore {
    /// Finalize at most 32 ended windows and return how many were due. A window whose
    /// read count changed since it was read stays for the next pass, and replicas racing
    /// for one window finalize it once.
    pub async fn close_indexing_windows(&self) -> Result<usize, AuditStoreError> {
        let windows = sqlx::query_file!("src/store/indexing/due.sql")
            .fetch_all(self.pool())
            .await?;
        let count = windows.len();
        for window in windows {
            let template: AuditDraft = self
                .targets()
                .decoder()
                .from_value(window.template)
                .map_err(|error| match error {
                    AuditDecodeError::Target(error) => AuditStoreError::Target(error),
                    _ => AuditStoreError::Integrity,
                })?;
            let collection: CollectionId = window
                .collection
                .parse()
                .map_err(|_| AuditStoreError::Integrity)?;
            let counter = |value: i64| u64::try_from(value).map_err(|_| AuditStoreError::Integrity);
            let draft = AuditDraft::builder(
                template.request().clone(),
                template.target().clone(),
                AuditDetail::IndexingWindow {
                    collection,
                    start: window.start_at,
                    end: window.end_at,
                    reads: counter(window.reads)?,
                    not_modified: counter(window.not_modified)?,
                    failed: counter(window.failed)?,
                    members_digest: Sha256Digest::parse(window.members_digest)
                        .map_err(|_| AuditStoreError::Integrity)?,
                },
                AuditOutcome::Succeeded,
                AuditReason::Accepted,
            )
            .identity(template.id())
            .occurred_at(window.end_at)
            .actor(template.actor().ok_or(AuditStoreError::Integrity)?.clone())
            .authority(template.authority().clone())
            .build()?;
            let record = AuditTransactionWrite::new(self.targets(), draft)?;
            let mut attempt = 0;
            loop {
                let result = self
                    .close_window(&window.scope, window.start_at, window.reads, &record)
                    .await;
                match result {
                    Err(error) if error.retryable() && attempt < 7 => {
                        tokio::time::sleep(Duration::from_millis(2u64.pow(attempt))).await;
                        attempt += 1;
                    }
                    result => break result?,
                }
            }
        }
        Ok(count)
    }

    async fn close_window(
        &self,
        scope: &str,
        start: DateTime<Utc>,
        reads: i64,
        record: &AuditTransactionWrite,
    ) -> Result<(), AuditStoreError> {
        let mut transaction = self.pool().begin().await?;
        let closed = sqlx::query_file!("src/store/indexing/close.sql", scope, start, reads)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
        if closed == 1 {
            record.append(&mut transaction).await?;
            transaction.commit().await?;
        } else {
            transaction.rollback().await?;
        }
        Ok(())
    }

    /// Remove up to 1,024 receipts older than a day and return how many were removed.
    /// Receipts outlive the hour in which a read is admitted, so removing one never lets
    /// an expired retry in.
    pub async fn prune_indexing_receipts(&self) -> Result<usize, AuditStoreError> {
        let removed = sqlx::query_file!("src/store/indexing/prune_receipts.sql")
            .execute(self.pool())
            .await?
            .rows_affected();
        usize::try_from(removed).map_err(|_| AuditStoreError::Integrity)
    }
}
