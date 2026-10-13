//! A drained signing-key transition across a sealer handoff, with externally anchored
//! verification and no installation secrets.
mod support;

use std::{collections::BTreeSet, num::NonZeroU32, sync::Arc, time::Duration};
use support::{AuditDb, archive, draft, state, task_cancel, within};
use veoveo_audit::{
    integrity::{AuditKeyRing, AuditSigningKey, AuditVerifier, IntegrityError},
    sealer::AuditSealer,
    *,
};

fn record() -> AuditDraft {
    draft(task_cancel())
}

fn verifier(keys: &AuditKeyRing, anchor: Option<AuditCheckpoint>) -> AuditVerifier<'_> {
    AuditVerifier::new(
        keys,
        AuditPartition::Installation,
        anchor,
        chrono::TimeDelta::seconds(30),
    )
    .unwrap()
}

#[tokio::test]
async fn drained_key_transition_preserves_the_archive_and_requires_post_cutover_trust() {
    within(Duration::from_secs(90), async {
        let audit = AuditDb::new().await;
        let scope = AuditReadScope::new(None, true);
        let partition = AuditPartition::Installation;
        let old = Arc::new(AuditSigningKey::from_seed(&[61; 32]));
        let replacement = Arc::new(AuditSigningKey::from_seed(&[62; 32]));
        let first = AuditService::start(
            audit.a.clone(),
            old.clone(),
            NonZeroU32::new(1).unwrap(),
            Default::default(),
        )
        .unwrap();
        let writer = AuditWriter::start(audit.a.clone());
        let before = (0..3).map(|_| record()).collect::<Vec<_>>();
        for draft in &before {
            writer.record(draft.clone()).await.unwrap();
        }
        let final_old = record();
        writer.record_completion(final_old.clone()).await;
        writer.shutdown(Duration::from_secs(10)).await.unwrap();
        first.shutdown(Duration::from_secs(15)).await.unwrap();
        assert!(matches!(
            writer.record(record()).await,
            Err(AuditWriteError::Closed)
        ));

        let prefix = archive(&audit.b, &scope, &partition).await;
        assert!(!prefix.is_empty());
        assert!(prefix.iter().all(|(block, _)| block.head.key_id == old.key_id()));
        assert_eq!(
            prefix
                .iter()
                .flat_map(|(_, records)| records.iter().map(|record| record.draft.id()))
                .collect::<BTreeSet<_>>(),
            before
                .iter()
                .chain(std::iter::once(&final_old))
                .map(AuditDraft::id)
                .collect()
        );
        // In operation this checkpoint comes from separately protected storage, not
        // from the database or the historical signing seed.
        let cutover = prefix.last().unwrap().0.checkpoint();
        let second = AuditService::start(
            audit.b.clone(),
            replacement.clone(),
            NonZeroU32::new(1).unwrap(),
            Default::default(),
        )
        .unwrap();
        let mut health = second.health();
        state(&mut health, AuditHealthState::Active).await;
        let contender = tokio::spawn({
            let audit = audit.a.clone();
            let old = old.clone();
            async move { AuditSealer::acquire(audit, old).await }
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !contender.is_finished(),
            "an old-key replica waits while the replacement holds the role"
        );
        let writer = AuditWriter::start(audit.b.clone());
        let after = (0..3).map(|_| record()).collect::<Vec<_>>();
        for draft in &after {
            writer.record(draft.clone()).await.unwrap();
        }
        writer.shutdown(Duration::from_secs(10)).await.unwrap();
        second.shutdown(Duration::from_secs(15)).await.unwrap();
        within(Duration::from_secs(10), contender)
            .await
            .unwrap()
            .unwrap()
            .release()
            .await
            .unwrap();

        let complete = archive(&audit.a, &scope, &partition).await;
        assert_eq!(
            &complete[..prefix.len()],
            prefix.as_slice(),
            "the transition rewrote retained blocks or records"
        );
        let suffix = &complete[prefix.len()..];
        assert!(!suffix.is_empty());
        assert!(
            suffix
                .iter()
                .all(|(block, _)| block.head.key_id == replacement.key_id())
        );
        assert_eq!(
            suffix
                .iter()
                .flat_map(|(_, records)| records.iter().map(|record| record.draft.id()))
                .collect::<BTreeSet<_>>(),
            after.iter().map(AuditDraft::id).collect()
        );
        let expected_tail = complete.last().unwrap().0.checkpoint();
        let mut archive_keys = AuditKeyRing::default();
        archive_keys.insert(old.public_key()).unwrap();
        archive_keys.insert(replacement.public_key()).unwrap();
        let mut retained = verifier(&archive_keys, None);
        for (block, records) in &prefix {
            assert!(retained.verify(block, records).unwrap().is_empty());
        }
        retained.finish(&cutover).unwrap();
        let mut full = verifier(&archive_keys, None);
        for (block, records) in &complete {
            assert!(full.verify(block, records).unwrap().is_empty());
        }
        full.finish(&expected_tail).unwrap();

        let mut current_keys = AuditKeyRing::default();
        current_keys.insert(replacement.public_key()).unwrap();
        let mut current = verifier(&current_keys, Some(cutover.clone()));
        for (block, records) in suffix {
            assert!(current.verify(block, records).unwrap().is_empty());
        }
        current.finish(&expected_tail).unwrap();
        assert!(matches!(
            verifier(&current_keys, Some(cutover.clone())).finish(&expected_tail),
            Err(IntegrityError::Incomplete)
        ));
        let (tail, records) = &suffix[0];
        assert!(
            matches!(
                verifier(&current_keys, None).verify(tail, records),
                Err(IntegrityError::Chain)
            ),
            "a removed prefix requires an independently trusted anchor"
        );
        assert!(matches!(
            verifier(&current_keys, None).verify(&prefix[0].0, &prefix[0].1),
            Err(IntegrityError::UnknownKey)
        ));
        let mut missing = AuditKeyRing::default();
        missing.insert(old.public_key()).unwrap();
        assert!(matches!(
            verifier(&missing, Some(cutover.clone())).verify(tail, records),
            Err(IntegrityError::UnknownKey)
        ));
        let mut wrong = AuditKeyRing::default();
        wrong
            .insert(AuditSigningKey::from_seed(&[63; 32]).public_key())
            .unwrap();
        assert!(matches!(
            verifier(&wrong, Some(cutover.clone())).verify(tail, records),
            Err(IntegrityError::UnknownKey)
        ));
        let forged = old
            .seal(partition, Some(&cutover), records, tail.head.sealed_at)
            .unwrap();
        // Archive membership does not revoke the old key. The replacement-only profile
        // refuses a valid historical-key signature beyond the trusted anchor.
        verifier(&archive_keys, Some(cutover.clone()))
            .verify(&forged, records)
            .unwrap();
        assert!(
            matches!(
                verifier(&current_keys, Some(cutover.clone())).verify(&forged, records),
                Err(IntegrityError::UnknownKey)
            ),
            "a compromised historical key authorized a new suffix"
        );
        let mut changed = records.clone();
        changed[0].recorded_at += chrono::TimeDelta::seconds(1);
        assert!(matches!(
            verifier(&current_keys, Some(cutover.clone())).verify(tail, &changed),
            Err(IntegrityError::RecordHash)
        ));
        let mut tampered = tail.clone();
        tampered.head.sealed_at += chrono::TimeDelta::seconds(1);
        assert!(matches!(
            verifier(&current_keys, Some(cutover)).verify(&tampered, records),
            Err(IntegrityError::HeadHash)
        ));
        println!(
            "{{\"schema\":\"https://veoveo.ai/formats/audit-key-transition-qualification\",\"retainedRecords\":{},\"replacementRecords\":{},\"sequence\":{}}}",
            before.len() + 1,
            after.len(),
            expected_tail.sequence.get()
        );
    })
    .await;
}
