//! Group commits, transactional appends and stored-record integrity.
mod support;

use std::time::Duration;
use support::{AuditDb, draft, note, task_update, user, within};
use veoveo_audit::*;

fn read(count: u32) -> AuditDraft {
    draft(note(count))
}

fn all() -> (AuditReadScope, AuditQuery) {
    let mut query = AuditQuery::new(AuditPartition::Installation);
    query.limit = 1000;
    (AuditReadScope::new(None, true), query)
}

/// The same identity and occurrence with another detail.
fn conflicting(original: &AuditDraft) -> AuditDraft {
    AuditDraft::builder(
        original.request().clone(),
        original.target().clone(),
        task_update(),
        AuditOutcome::Allowed,
        AuditReason::Accepted,
    )
    .identity(original.id())
    .occurred_at(original.occurred_at())
    .build()
    .unwrap()
}

#[tokio::test]
async fn groups_commit_across_replicas_and_a_conflicting_identity_rolls_back_the_group() {
    within(Duration::from_secs(90), async {
        let audit = AuditDb::new().await;
        let first = (0..65).map(|_| read(1)).collect::<Vec<_>>();
        let second = (0..65).map(|_| read(2)).collect::<Vec<_>>();
        let (a, b) = tokio::join!(audit.a.append(&first), audit.b.append(&second));
        a.unwrap();
        b.unwrap();
        let (scope, query) = all();
        let stored = audit.a.page(&scope, &query).await.unwrap();
        assert!(stored.next.is_none());
        assert_eq!(stored.records.len(), 130);
        for expected in first.iter().chain(&second) {
            let record = stored
                .records
                .iter()
                .find(|record| record.draft.id() == expected.id())
                .unwrap();
            assert_eq!(&record.draft, expected);
        }
        audit.b.append(&first).await.unwrap();
        assert_eq!(
            audit.a.page(&scope, &query).await.unwrap().records.len(),
            130,
            "an identical retry changes nothing"
        );
        let pending = read(1);
        assert!(matches!(
            audit
                .a
                .append(&[pending.clone(), conflicting(&first[0])])
                .await,
            Err(AuditStoreError::IdentityConflict)
        ));
        let after = audit.b.page(&scope, &query).await.unwrap();
        assert_eq!(after.records.len(), 130);
        assert!(
            after
                .records
                .iter()
                .all(|record| record.draft.id() != pending.id())
        );
    })
    .await;
}

#[tokio::test]
async fn domain_writes_and_audit_records_commit_or_roll_back_together() {
    within(Duration::from_secs(90), async {
        let audit = AuditDb::new().await;
        let pool = audit.db.a.pool();
        sqlx::query(include_str!("queries/append/domain_table.sql"))
            .execute(pool)
            .await
            .unwrap();
        let domain_rows = || async {
            sqlx::query_scalar::<_, i64>(include_str!("queries/append/domain_count.sql"))
                .fetch_one(audit.db.b.pool())
                .await
                .unwrap()
        };
        let (scope, query) = all();

        let committed = read(1);
        let write = AuditTransactionWrite::new(audit.a.registry(), committed.clone()).unwrap();
        let mut transaction = pool.begin().await.unwrap();
        sqlx::query(include_str!("queries/append/domain_insert.sql"))
            .bind(1)
            .execute(&mut *transaction)
            .await
            .unwrap();
        write.append(&mut transaction).await.unwrap();
        assert!(
            audit
                .b
                .page(&scope, &query)
                .await
                .unwrap()
                .records
                .is_empty(),
            "another replica sees nothing before the domain commit"
        );
        transaction.commit().await.unwrap();
        assert_eq!(domain_rows().await, 1);
        let stored = audit.b.page(&scope, &query).await.unwrap().records;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].draft, committed);

        let mut retry = pool.begin().await.unwrap();
        write.append(&mut retry).await.unwrap();
        retry.commit().await.unwrap();
        assert_eq!(audit.b.page(&scope, &query).await.unwrap().records.len(), 1);

        let mut transaction = pool.begin().await.unwrap();
        sqlx::query(include_str!("queries/append/domain_insert.sql"))
            .bind(2)
            .execute(&mut *transaction)
            .await
            .unwrap();
        AuditTransactionWrite::new(audit.a.registry(), read(2))
            .unwrap()
            .append(&mut transaction)
            .await
            .unwrap();
        transaction.rollback().await.unwrap();
        assert_eq!(domain_rows().await, 1);
        assert_eq!(audit.b.page(&scope, &query).await.unwrap().records.len(), 1);

        let mut transaction = pool.begin().await.unwrap();
        sqlx::query(include_str!("queries/append/domain_insert.sql"))
            .bind(3)
            .execute(&mut *transaction)
            .await
            .unwrap();
        let conflict =
            AuditTransactionWrite::batch(audit.a.registry(), [read(3), conflicting(&committed)])
                .unwrap();
        assert!(matches!(
            conflict.append(&mut transaction).await,
            Err(AuditAppendError::IdentityConflict)
        ));
        assert!(
            sqlx::query(include_str!("queries/append/domain_insert.sql"))
                .bind(4)
                .execute(&mut *transaction)
                .await
                .is_err(),
            "a conflicting identity aborts the caller's transaction"
        );
        let _ = transaction.commit().await;
        assert_eq!(domain_rows().await, 1);
        assert_eq!(audit.b.page(&scope, &query).await.unwrap().records.len(), 1);

        let limit = (0..=AUDIT_APPEND_LIMIT).map(|_| read(2));
        assert!(matches!(
            AuditTransactionWrite::batch(audit.a.registry(), limit),
            Err(AuditAppendError::BatchLimit)
        ));
    })
    .await;
}

#[tokio::test]
async fn stored_records_are_immutable_and_readers_reject_rows_that_disagree_with_their_draft() {
    within(Duration::from_secs(90), async {
        let audit = AuditDb::new().await;
        let alice = user("alpha", "alice");
        let original = AuditDraft::builder(
            AuditRequest::background(),
            AuditTarget::Installation,
            note(1),
            AuditOutcome::Allowed,
            AuditReason::Accepted,
        )
        .actor(alice.clone())
        .authority(AuditAuthority {
            work_context: Some("operations".parse().unwrap()),
            ..Default::default()
        })
        .build()
        .unwrap();
        audit
            .a
            .append(std::slice::from_ref(&original))
            .await
            .unwrap();
        let columns = sqlx::query_file!(
            "tests/queries/append/stored_columns.sql",
            original.id() as AuditRecordId
        )
        .fetch_one(audit.db.b.pool())
        .await
        .unwrap();
        assert_eq!(columns.partition, "tenant:alpha");
        assert_eq!(columns.outcome, "allowed");
        assert_eq!(columns.actor.as_deref(), Some("alice"));
        assert_eq!(columns.class, "fixture.note");
        assert_eq!(columns.trace_id, original.request().trace_id.as_str());
        assert_eq!(
            columns.day,
            original
                .occurred_at()
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
        );
        assert!(columns.txid.parse::<u64>().unwrap() > 0);

        assert!(
            sqlx::query_file!(
                "tests/queries/append/reclassify.sql",
                original.id() as AuditRecordId
            )
            .execute(audit.db.a.pool())
            .await
            .is_err(),
            "stored records are immutable"
        );
        let partition = AuditPartition::Tenant("alpha".parse().unwrap());
        let scope = AuditReadScope::new(Some("alpha".parse().unwrap()), false);
        let query = AuditQuery::new(partition.clone());
        let page = audit.b.page(&scope, &query).await.unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].draft, original);
        assert_eq!(page.records[0].recorded_at, columns.recorded_at);

        // A privileged writer can still store a row whose occurrence time disagrees with
        // its draft. Every reader rejects it after SQL selection.
        sqlx::query_file!(
            "tests/queries/append/insert_mistimed.sql",
            original.id() as AuditRecordId,
            AuditRecordId::new() as AuditRecordId,
        )
        .execute(audit.db.a.pool())
        .await
        .unwrap();
        assert!(matches!(
            audit.b.page(&scope, &query).await,
            Err(AuditStoreError::Integrity)
        ));
    })
    .await;
}

#[tokio::test]
async fn the_writer_acknowledges_after_commit_and_refuses_records_after_shutdown() {
    within(Duration::from_secs(90), async {
        let audit = AuditDb::new().await;
        let writer = AuditWriter::start(audit.a.clone());
        let scope = AuditReadScope::new(None, true);
        futures::future::join_all((0..16).map(|_| {
            let record = read(2);
            let writer = &writer;
            let reader = &audit.b;
            let scope = &scope;
            async move {
                writer.record(record.clone()).await.unwrap();
                let mut query = AuditQuery::new(AuditPartition::Installation);
                query.trace = Some(record.request().trace_id.clone());
                let page = reader.page(scope, &query).await.unwrap();
                assert_eq!(
                    page.records.len(),
                    1,
                    "an acknowledgement follows the commit"
                );
                assert_eq!(page.records[0].draft, record);
            }
        }))
        .await;
        let completion = read(3);
        writer.record_completion(completion.clone()).await;
        writer.shutdown(Duration::from_secs(10)).await.unwrap();
        assert!(matches!(
            writer.record(read(2)).await,
            Err(AuditWriteError::Closed)
        ));
        let (_, query) = all();
        let stored = audit.b.page(&scope, &query).await.unwrap().records;
        assert_eq!(stored.len(), 17);
        assert!(
            stored
                .iter()
                .any(|record| record.draft.id() == completion.id())
        );
    })
    .await;
}
