//! Indexing reads accumulate in database-clock windows that survive writer replacement,
//! deduplicate retries and close once across replicas.
mod support;

use sha2::{Digest, Sha256};
use std::{num::NonZeroU32, sync::Arc, time::Duration};
use support::{AuditDb, within};
use veoveo_audit::{integrity::AuditSigningKey, *};
use veoveo_mcp_ext_knowledge::{Observation, Revision, content_digest, docs};

fn indexer(tenant: &str) -> AuditActor {
    AuditActor {
        principal: "indexer".parse().unwrap(),
        kind: AuditPrincipalKind::Service,
        tenant: Some(tenant.parse().unwrap()),
        oauth_client: Some("indexer".parse().unwrap()),
        session_family: None,
        delegating_principal: None,
        managed_agent: None,
    }
}

fn read(not_modified: bool, failed: bool) -> IndexingRead {
    let server = "time".parse().unwrap();
    let collection = docs::collection(&server, &"time".parse().unwrap());
    let uri = veoveo_types::ResourceUri::new("time://docs/design").unwrap();
    let observation = Observation::builder(
        collection.collection().clone(),
        Revision::parse("revision-1").unwrap(),
        content_digest("body"),
        chrono::Utc::now(),
    )
    .build(&collection)
    .unwrap();
    let mut observed: KnowledgeReadObservation = (&observation).into();
    observed.not_modified = not_modified;
    let (detail, outcome, reason) = if failed {
        (
            AuditDetail::Read {
                method: AuditReadMethod::ResourceRead,
            },
            AuditOutcome::Failed,
            AuditReason::UpstreamFailure,
        )
    } else {
        (
            AuditDetail::KnowledgeRead {
                member: uri.clone(),
                observation: Some(Box::new(observed)),
                status: if not_modified {
                    KnowledgeReadStatus::NotModified
                } else {
                    KnowledgeReadStatus::Read
                },
            },
            AuditOutcome::Succeeded,
            AuditReason::Accepted,
        )
    };
    let draft = AuditDraft::builder(
        AuditRequest::background(),
        AuditTarget::Resource { server, uri },
        detail,
        outcome,
        reason,
    )
    .actor(indexer("test-tenant"))
    .build()
    .unwrap();
    IndexingRead::new(draft, collection.collection().clone()).unwrap()
}

fn tenant() -> (AuditReadScope, AuditPartition) {
    (
        AuditReadScope::new(Some("test-tenant".parse().unwrap()), false),
        AuditPartition::Tenant("test-tenant".parse().unwrap()),
    )
}

async fn records(store: &AuditStore) -> Vec<AuditRecord> {
    let (scope, partition) = tenant();
    store
        .page(&scope, &AuditQuery::new(partition))
        .await
        .unwrap()
        .records
}

async fn window_reads(audit: &AuditDb) -> Vec<i64> {
    sqlx::query_file_scalar!("tests/queries/indexing/window_reads.sql")
        .fetch_all(audit.db.b.pool())
        .await
        .unwrap()
}

/// Move open windows into the previous five minutes. The finalizer still uses the
/// database clock and the immutable append path.
async fn expire_windows(audit: &AuditDb) {
    sqlx::query_file!("tests/queries/indexing/expire_windows.sql")
        .execute(audit.db.a.pool())
        .await
        .unwrap();
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    veoveo_types::Sha256Digest::from_bytes(Sha256::digest(bytes).into())
        .hex()
        .to_owned()
}

#[tokio::test]
async fn acknowledged_reads_survive_writer_replacement_and_their_window_is_sealed() {
    within(Duration::from_secs(90), async {
        let audit = AuditDb::new().await;
        let (scope, partition) = tenant();
        let read = read(false, false);
        let writer = AuditWriter::start(audit.a.clone());
        writer.record_indexing(read.clone()).await.unwrap();
        assert_eq!(
            window_reads(&audit).await,
            vec![1],
            "an acknowledgement follows the committed accumulation"
        );
        writer.shutdown(Duration::from_secs(10)).await.unwrap();
        assert!(matches!(
            writer.record_indexing(read.clone()).await,
            Err(AuditWriteError::Closed)
        ));
        expire_windows(&audit).await;
        let successor = AuditWriter::start(audit.b.clone());
        let service = AuditService::start(
            audit.a.clone(),
            Arc::new(AuditSigningKey::from_seed(&[37; 32])),
            NonZeroU32::new(1).unwrap(),
            Default::default(),
        )
        .unwrap();
        let summary = within(Duration::from_secs(15), async {
            loop {
                if let Some(record) = records(&audit.a).await.into_iter().next() {
                    break record;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await;
        assert!(matches!(
            summary.draft.detail(),
            AuditDetail::IndexingWindow { reads: 1, .. }
        ));
        successor.record_indexing(read).await.unwrap();
        within(
            Duration::from_secs(10),
            audit.a.wait_sealed(&scope, &partition, summary.draft.id()),
        )
        .await
        .unwrap();
        successor.shutdown(Duration::from_secs(10)).await.unwrap();
        service.shutdown(Duration::from_secs(15)).await.unwrap();
        assert_eq!(records(&audit.a).await.len(), 1);
    })
    .await;
}

#[tokio::test]
async fn windows_deduplicate_retries_separate_authority_and_close_once_across_replicas() {
    within(Duration::from_secs(90), async {
        let audit = AuditDb::new().await;
        let reads = vec![read(false, false), read(true, false), read(false, true)];
        let (a, b) = tokio::join!(
            audit.a.append_group(&[], &reads),
            audit.b.append_group(&[], &reads)
        );
        a.unwrap();
        b.unwrap();
        assert!(records(&audit.b).await.is_empty(), "no record per read");
        assert_eq!(
            audit.b.close_indexing_windows().await.unwrap(),
            0,
            "an open window stays open"
        );
        audit.b.append_group(&[], &reads).await.unwrap();
        let mut altered = serde_json::to_value(reads[0].draft()).unwrap();
        altered["authority"]["profile"] = serde_json::json!("other");
        let conflict = IndexingRead::new(
            audit
                .a
                .targets()
                .decoder()
                .from_value(altered.clone())
                .unwrap(),
            reads[0].collection().clone(),
        )
        .unwrap();
        assert!(
            matches!(
                audit
                    .b
                    .append_group(
                        std::slice::from_ref(reads[0].draft()),
                        &[read(false, false), conflict]
                    )
                    .await,
                Err(AuditStoreError::IdentityConflict)
            ),
            "a changed retry rolls back the whole group"
        );
        altered["id"] = serde_json::to_value(AuditRecordId::new()).unwrap();
        let other = IndexingRead::new(
            audit.a.targets().decoder().from_value(altered).unwrap(),
            reads[0].collection().clone(),
        )
        .unwrap();
        audit.a.append_group(&[], &[other]).await.unwrap();
        expire_windows(&audit).await;
        let (a, b) = tokio::join!(
            audit.a.close_indexing_windows(),
            audit.b.close_indexing_windows()
        );
        a.unwrap();
        b.unwrap();
        let sealed = records(&audit.b).await;
        assert_eq!(
            sealed.len(),
            2,
            "authority changes do not merge attribution"
        );
        let summary = sealed
            .iter()
            .find(|record| record.draft.authority().profile.is_none())
            .unwrap();
        let AuditDetail::IndexingWindow {
            reads: count,
            not_modified,
            failed,
            members_digest,
            ..
        } = summary.draft.detail()
        else {
            panic!("an indexing window was expected")
        };
        assert_eq!((*count, *not_modified, *failed), (3, 1, 1));
        let mut digest = hex(b"https://veoveo.ai/formats/audit-indexing-members");
        for _ in 0..2 {
            let leaf = hex(serde_json::to_vec(&("time://docs/design", "revision-1")).unwrap());
            digest = hex(format!(
                "https://veoveo.ai/formats/audit-indexing-members:{digest}{leaf}"
            ));
        }
        assert_eq!(members_digest.hex(), digest);
        audit.b.append_group(&[], &reads).await.unwrap();
        assert_eq!(
            audit.a.close_indexing_windows().await.unwrap(),
            0,
            "a retry after the close cannot reopen the window"
        );
        assert_eq!(records(&audit.a).await.len(), 2);
        // A two-day-old read is refused even when no receipt remembers it.
        let mut expired = serde_json::to_value(reads[0].draft()).unwrap();
        expired["id"] = serde_json::to_value(AuditRecordId::new()).unwrap();
        expired["occurred_at"] =
            serde_json::to_value(chrono::Utc::now() - chrono::Duration::days(2)).unwrap();
        let expired = IndexingRead::new(
            audit.a.targets().decoder().from_value(expired).unwrap(),
            reads[0].collection().clone(),
        )
        .unwrap();
        assert!(matches!(
            audit.a.append_group(&[], &[expired]).await,
            Err(AuditStoreError::IndexingReadExpired)
        ));
        assert_eq!(audit.a.prune_indexing_receipts().await.unwrap(), 0);
        // Cleanup is bounded, replica-safe and never removes an open window.
        audit
            .a
            .append_group(&[], &[read(false, false)])
            .await
            .unwrap();
        sqlx::query_file!("tests/queries/indexing/expired_receipts.sql", 1030)
            .execute(audit.db.a.pool())
            .await
            .unwrap();
        let (a, b) = tokio::join!(
            audit.a.prune_indexing_receipts(),
            audit.b.prune_indexing_receipts()
        );
        let first = a.unwrap();
        let second = b.unwrap();
        assert!(first <= 1024 && second <= 1024);
        let rest = audit.a.prune_indexing_receipts().await.unwrap();
        assert_eq!(first + second + rest, 1030);
        assert_eq!(audit.a.prune_indexing_receipts().await.unwrap(), 0);
        assert_eq!(window_reads(&audit).await, vec![1]);
    })
    .await;
}
