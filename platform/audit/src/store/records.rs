//! Appends and partition-scoped record reads. Policy admission and target registry
//! checks run before SQL, and every returned row is admitted before it is returned.
use super::{AuditStore, AuditStoreError, admit, indexing::IndexingRows, row::RecordRow};
use std::time::Duration;
use veoveo_audit_contract::{
    AUDIT_APPEND_LIMIT, AuditClass, AuditCursor, AuditDailyCount, AuditDailyCursor, AuditDailyPage,
    AuditDailyQuery, AuditDetail, AuditDraft, AuditOrder, AuditOutcome, AuditPage, AuditPartition,
    AuditQuery, AuditReadMethod, AuditReadScope, AuditRecord, AuditRecordId, AuditTarget,
    AuditTraceId, AuditTransactionWrite, IndexingRead,
};
use veoveo_types::{GatewayProfileId, PrincipalId};

impl AuditStore {
    /// Append drafts in one transaction of their own and return once it commits.
    pub async fn append(&self, drafts: &[AuditDraft]) -> Result<(), AuditStoreError> {
        self.append_group(drafts, &[]).await
    }

    /// Commit records and indexing reads together. A serialization failure or deadlock
    /// repeats the whole transaction up to three more times.
    pub async fn append_group(
        &self,
        drafts: &[AuditDraft],
        indexing: &[IndexingRead],
    ) -> Result<(), AuditStoreError> {
        if drafts.is_empty() && indexing.is_empty() {
            return Ok(());
        }
        if indexing.len() > AUDIT_APPEND_LIMIT {
            return Err(AuditStoreError::BatchLimit);
        }
        let records = AuditTransactionWrite::batch(self.targets(), drafts.iter().cloned())?;
        let reads = IndexingRows::encode(self.targets(), indexing)?;
        let mut attempt = 0;
        loop {
            match self.append_once(&records, &reads).await {
                Err(error) if error.retryable() && attempt < 3 => {
                    tokio::time::sleep(Duration::from_millis(2u64.pow(attempt))).await;
                    attempt += 1;
                }
                result => return result,
            }
        }
    }

    async fn append_once(
        &self,
        records: &AuditTransactionWrite,
        reads: &IndexingRows,
    ) -> Result<(), AuditStoreError> {
        let mut transaction = self.pool().begin().await?;
        records.append(&mut transaction).await?;
        reads.append(&mut transaction).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// One page of a partition. The cursor carries the partition and order it was
    /// issued for; a page holds the records after it. The page plans with its actual
    /// filters, so an actor, trace or time filter seeks its index.
    pub async fn page(
        &self,
        scope: &AuditReadScope,
        query: &AuditQuery,
    ) -> Result<AuditPage, AuditStoreError> {
        query.validate()?;
        if let Some(target) = &query.target {
            self.targets().validate(target)?;
        }
        admit(scope, &query.partition)?;
        let partition = query.partition.storage_key();
        let after = query.cursor.as_ref().map(|cursor| cursor.last_id);
        let actor = query.actor.as_ref().map(PrincipalId::as_str);
        let target = query.target.as_ref().map(target_document);
        let trace = query.trace.as_ref();
        let limit = i64::from(query.limit) + 1;
        let mut transaction = self.pool().begin().await?;
        sqlx::query_file!("src/store/records/custom_plans.sql")
            .execute(&mut *transaction)
            .await?;
        let rows = match query.order {
            AuditOrder::OldestFirst => {
                sqlx::query_file_as!(
                    RecordRow,
                    "src/store/records/page_oldest_first.sql",
                    partition,
                    after as Option<AuditRecordId>,
                    query.class as Option<AuditClass>,
                    actor,
                    target,
                    query.outcome as Option<AuditOutcome>,
                    trace as Option<&AuditTraceId>,
                    query.from,
                    query.until,
                    limit,
                )
                .fetch_all(&mut *transaction)
                .await?
            }
            AuditOrder::NewestFirst => {
                sqlx::query_file_as!(
                    RecordRow,
                    "src/store/records/page_newest_first.sql",
                    partition,
                    after as Option<AuditRecordId>,
                    query.class as Option<AuditClass>,
                    actor,
                    target,
                    query.outcome as Option<AuditOutcome>,
                    trace as Option<&AuditTraceId>,
                    query.from,
                    query.until,
                    limit,
                )
                .fetch_all(&mut *transaction)
                .await?
            }
        };
        transaction.commit().await?;
        let mut records = rows
            .into_iter()
            .map(|row| row.checked(self.targets(), &query.partition))
            .collect::<Result<Vec<_>, _>>()?;
        let more = records.len() > usize::from(query.limit);
        records.truncate(usize::from(query.limit));
        let next = more
            .then(|| records.last())
            .flatten()
            .map(|record| AuditCursor {
                order: query.order,
                partition: query.partition.clone(),
                last_id: record.draft.id(),
            });
        Ok(AuditPage { records, next })
    }

    /// One page of a partition's counts per UTC day, class and outcome.
    pub async fn daily(
        &self,
        scope: &AuditReadScope,
        query: &AuditDailyQuery,
    ) -> Result<AuditDailyPage, AuditStoreError> {
        admit(scope, &query.partition)?;
        query.validate()?;
        let cursor = query.cursor.as_ref();
        let rows = sqlx::query_file!(
            "src/store/records/daily.sql",
            query.partition.storage_key(),
            query.from,
            query.until,
            cursor.map(|cursor| cursor.day),
            cursor.map(|cursor| cursor.class) as Option<AuditClass>,
            cursor.map(|cursor| cursor.outcome) as Option<AuditOutcome>,
            i64::from(query.limit) + 1,
        )
        .fetch_all(self.pool())
        .await?;
        let mut counts = rows
            .into_iter()
            .map(|row| {
                Ok(AuditDailyCount {
                    partition: query.partition.clone(),
                    day: row.day,
                    class: row.class,
                    outcome: row.outcome,
                    count: u64::try_from(row.count).map_err(|_| AuditStoreError::Integrity)?,
                })
            })
            .collect::<Result<Vec<_>, AuditStoreError>>()?;
        let more = counts.len() > usize::from(query.limit);
        counts.truncate(usize::from(query.limit));
        let next = more
            .then(|| counts.last())
            .flatten()
            .map(|count| AuditDailyCursor {
                partition: query.partition.clone(),
                day: count.day,
                class: count.class,
                outcome: count.outcome,
            });
        Ok(AuditDailyPage { counts, next })
    }

    /// Whether `view` is the caller's own committed audit-view receipt for `partition`,
    /// issued under `profile` within the last fifteen minutes. The receipt identity
    /// grants nothing by itself.
    pub async fn view_admitted(
        &self,
        scope: &AuditReadScope,
        partition: &AuditPartition,
        actor_partition: &AuditPartition,
        actor: &PrincipalId,
        profile: &GatewayProfileId,
        view: AuditRecordId,
    ) -> Result<bool, AuditStoreError> {
        admit(scope, partition)?;
        admit(scope, actor_partition)?;
        let target = AuditTarget::AuditLog {
            partition: partition.clone(),
        };
        let detail = AuditDetail::Read {
            method: AuditReadMethod::AuditView,
        };
        let row = sqlx::query_file_as!(
            RecordRow,
            "src/store/records/view.sql",
            view as AuditRecordId,
            actor_partition.storage_key(),
            actor.as_str(),
            profile.as_str(),
            target_document(&target),
            serde_json::to_value(&detail).expect("a typed audit detail serializes"),
        )
        .fetch_optional(self.pool())
        .await?;
        let Some(row) = row else {
            return Ok(false);
        };
        let record = row.checked(self.targets(), actor_partition)?;
        let draft = &record.draft;
        if draft.actor().map(|actor| &actor.principal) != Some(actor)
            || draft.authority().profile.as_ref() != Some(profile)
            || draft.target() != &target
            || draft.detail() != &detail
            || draft.outcome() != AuditOutcome::Allowed
        {
            return Err(AuditStoreError::Integrity);
        }
        Ok(true)
    }

    /// The partition's records among `ids`, keyed by identity.
    pub(crate) async fn records_by_id(
        &self,
        partition: &AuditPartition,
        ids: &[AuditRecordId],
    ) -> Result<std::collections::BTreeMap<AuditRecordId, AuditRecord>, AuditStoreError> {
        if ids.len() > AUDIT_APPEND_LIMIT {
            return Err(AuditStoreError::BatchLimit);
        }
        let rows = sqlx::query_file_as!(
            RecordRow,
            "src/store/records/by_id.sql",
            partition.storage_key(),
            ids as &[AuditRecordId],
        )
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|row| {
                let record = row.checked(self.targets(), partition)?;
                Ok((record.draft.id(), record))
            })
            .collect()
    }
}

/// A target as the stored draft holds it, for whole-object comparison in SQL.
pub(crate) fn target_document(target: &AuditTarget) -> serde_json::Value {
    serde_json::to_value(target).expect("an admitted audit target serializes")
}
