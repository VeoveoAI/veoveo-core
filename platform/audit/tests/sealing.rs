//! Commit-safe sealing order, gapless positions, role handoff and shutdown drains.
mod support;

use std::{num::NonZeroU32, sync::Arc, time::Duration};
use support::{AuditDb, archive, draft, seal_all, state, within};
use veoveo_audit::{
    integrity::{AuditKeyRing, AuditSigningKey, AuditVerifier},
    sealer::{AuditSealer, SealBacklog},
    *,
};

fn read(method: AuditReadMethod) -> AuditDraft {
    draft(AuditDetail::Read { method })
}

fn installation() -> (AuditReadScope, AuditPartition) {
    (
        AuditReadScope::new(None, true),
        AuditPartition::Installation,
    )
}

/// Every sealed member in chain order. Another database's open transaction on the same
/// cluster can split a pass, so tests compare positions rather than block boundaries.
fn positions(archived: &[(AuditBlock, Vec<AuditRecord>)]) -> Vec<(AuditRecordId, u64)> {
    archived
        .iter()
        .flat_map(|(block, _)| &block.head.members)
        .map(|member| (member.id, member.position.get()))
        .collect()
}

#[tokio::test]
async fn an_open_transaction_holds_back_records_that_commit_after_it() {
    within(Duration::from_secs(60), async {
        let audit = AuditDb::new().await;
        let (scope, partition) = installation();
        let sealer = AuditSealer::acquire(
            audit.a.clone(),
            Arc::new(AuditSigningKey::from_seed(&[3; 32])),
        )
        .await
        .unwrap();
        let earlier = read(AuditReadMethod::Status);
        let later = read(AuditReadMethod::Usage);
        // The earlier transaction writes its record first and stays open while a later
        // transaction commits.
        let mut open = audit.db.a.pool().begin().await.unwrap();
        AuditTransactionWrite::new(audit.a.targets(), earlier.clone())
            .unwrap()
            .append(&mut open)
            .await
            .unwrap();
        audit.b.append(std::slice::from_ref(&later)).await.unwrap();
        let pass = sealer.seal_pending().await.unwrap();
        assert_eq!(pass.sealed, 0, "a committed record cannot pass an open one");
        assert_eq!(pass.backlog, SealBacklog::Blocked);
        assert!(
            tokio::time::timeout(
                Duration::from_millis(300),
                audit.b.wait_sealed(&scope, &partition, later.id())
            )
            .await
            .is_err()
        );
        open.commit().await.unwrap();
        assert_eq!(seal_all(&sealer).await, 2);
        assert_eq!(
            positions(&archive(&audit.b, &scope, &partition).await),
            [(earlier.id(), 1), (later.id(), 2)]
        );
        audit
            .b
            .wait_sealed(&scope, &partition, later.id())
            .await
            .unwrap();
        sealer.release().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn positions_follow_transaction_order_without_gaps_across_blocks() {
    within(Duration::from_secs(60), async {
        let audit = AuditDb::new().await;
        let (scope, partition) = installation();
        let key = Arc::new(AuditSigningKey::from_seed(&[5; 32]));
        let sealer = AuditSealer::acquire(audit.a.clone(), key.clone())
            .await
            .unwrap();
        // The first transaction takes its ID before the second one writes, then inserts
        // its record after the second one committed. Its record has the higher
        // insertion sequence and still precedes the other in the partition.
        let first = read(AuditReadMethod::Status);
        let second = read(AuditReadMethod::Usage);
        let mut transaction = audit.db.a.pool().begin().await.unwrap();
        let first_txid = sqlx::query_file_scalar!("tests/queries/sealing/assign_txid.sql")
            .fetch_one(&mut *transaction)
            .await
            .unwrap();
        audit.b.append(std::slice::from_ref(&second)).await.unwrap();
        AuditTransactionWrite::new(audit.a.targets(), first.clone())
            .unwrap()
            .append(&mut transaction)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        assert!(first_txid.parse::<u64>().unwrap() > 0);
        assert_eq!(seal_all(&sealer).await, 2);

        let rest = (0..3)
            .map(|_| read(AuditReadMethod::ResourceRead))
            .collect::<Vec<_>>();
        for record in &rest {
            audit.a.append(std::slice::from_ref(record)).await.unwrap();
        }
        assert_eq!(seal_all(&sealer).await, 3);
        let archived = archive(&audit.b, &scope, &partition).await;
        assert!(archived.len() >= 2, "each pass seals its own block");
        assert_eq!(
            positions(&archived),
            [first.id(), second.id()]
                .into_iter()
                .chain(rest.iter().map(AuditDraft::id))
                .zip(1..)
                .collect::<Vec<_>>()
        );
        let checkpoint = audit
            .b
            .partition_checkpoint(&partition)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(checkpoint, archived.last().unwrap().0.checkpoint());
        assert_eq!(checkpoint.last_position.get(), 5);
        let mut keys = AuditKeyRing::default();
        keys.insert(key.public_key()).unwrap();
        let mut verifier =
            AuditVerifier::new(&keys, partition, None, chrono::TimeDelta::minutes(1)).unwrap();
        for (block, records) in &archived {
            assert!(verifier.verify(block, records).unwrap().is_empty());
        }
        verifier.finish(&checkpoint).unwrap();
        sealer.release().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn a_standby_takes_over_and_shutdown_seals_every_committed_record() {
    within(Duration::from_secs(90), async {
        let audit = AuditDb::new().await;
        let (scope, partition) = installation();
        let key = Arc::new(AuditSigningKey::from_seed(&[7; 32]));
        let peer = AuditSealer::acquire(audit.a.clone(), key.clone())
            .await
            .unwrap();
        let first = AuditService::start(
            audit.a.clone(),
            key.clone(),
            NonZeroU32::new(1).unwrap(),
            Default::default(),
        )
        .unwrap();
        let mut first_health = first.health();
        state(&mut first_health, AuditHealthState::Standby).await;
        assert!(
            first_health.ready(),
            "a replica waiting for the role is ready"
        );

        let backlog = (0..193)
            .map(|_| read(AuditReadMethod::ResourceRead))
            .collect::<Vec<_>>();
        audit.a.append(&backlog).await.unwrap();
        peer.release().await.unwrap();
        state(&mut first_health, AuditHealthState::Active).await;
        let second = AuditService::start(
            audit.b.clone(),
            key,
            NonZeroU32::new(1).unwrap(),
            Default::default(),
        )
        .unwrap();
        let mut second_health = second.health();
        state(&mut second_health, AuditHealthState::Standby).await;

        let writer = AuditWriter::start(audit.a.clone());
        for _ in 0..16 {
            writer.record(read(AuditReadMethod::Status)).await.unwrap();
        }
        writer.record_completion(read(AuditReadMethod::Usage)).await;
        writer.shutdown(Duration::from_secs(10)).await.unwrap();
        first.shutdown(Duration::from_secs(15)).await.unwrap();
        let sealed = archive(&audit.b, &scope, &partition)
            .await
            .iter()
            .map(|(_, records)| records.len())
            .sum::<usize>();
        assert_eq!(
            sealed,
            193 + 16 + 1,
            "shutdown seals every committed record"
        );

        state(&mut second_health, AuditHealthState::Active).await;
        let after_takeover = read(AuditReadMethod::Status);
        audit
            .a
            .append(std::slice::from_ref(&after_takeover))
            .await
            .unwrap();
        within(
            Duration::from_secs(10),
            audit.a.wait_sealed(&scope, &partition, after_takeover.id()),
        )
        .await
        .unwrap();
        second.shutdown(Duration::from_secs(15)).await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn shutdown_reports_a_failed_seal_and_keeps_the_record_unsealed() {
    within(Duration::from_secs(90), async {
        let audit = AuditDb::new().await;
        let (scope, partition) = installation();
        for statement in [
            include_str!("queries/sealing/fail_function.sql"),
            include_str!("queries/sealing/fail_trigger.sql"),
        ] {
            sqlx::query(statement)
                .execute(audit.db.a.pool())
                .await
                .unwrap();
        }
        let service = AuditService::start(
            audit.a.clone(),
            Arc::new(AuditSigningKey::from_seed(&[29; 32])),
            NonZeroU32::new(1).unwrap(),
            Default::default(),
        )
        .unwrap();
        state(&mut service.health(), AuditHealthState::Active).await;
        let record = read(AuditReadMethod::Status);
        audit.a.append(std::slice::from_ref(&record)).await.unwrap();
        assert!(
            matches!(
                service.shutdown(Duration::from_secs(15)).await,
                Err(AuditServiceError::Seal(_))
            ),
            "a failed drain does not report a clean shutdown"
        );
        let page = audit
            .b
            .page(&scope, &AuditQuery::new(partition.clone()))
            .await
            .unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].draft.id(), record.id());
        assert!(
            audit
                .b
                .blocks(&scope, &partition, None, 10)
                .await
                .unwrap()
                .is_empty()
        );
    })
    .await;
}
