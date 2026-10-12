//! One whole-block policy covers every record class and keeps unsealed records.
mod support;

use chrono::TimeDelta;
use std::{sync::Arc, time::Duration};
use support::{AuditDb, draft, seal_all, within};
use veoveo_audit::{integrity::AuditSigningKey, sealer::AuditSealer, *};

#[tokio::test]
async fn cutoff_retention_removes_every_class_only_after_its_block_expires() {
    within(Duration::from_secs(90), async {
        let audit = AuditDb::new().await;
        let scope = AuditReadScope::new(None, true);
        let partition = AuditPartition::Installation;
        let details = [
            AuditDetail::Read {
                method: AuditReadMethod::Status,
            },
            AuditDetail::Authentication {
                activity: AuthenticationActivity::Login,
                method: veoveo_types::AuthMethod::BearerJwt,
                reason: veoveo_types::AuthReasonCode::AuthAllow,
            },
            AuditDetail::AccountChange {
                activity: AccountActivity::Update,
            },
            AuditDetail::Artifact {
                activity: ArtifactActivity::Inspect,
                requested: None,
                subject: None,
                release_state: None,
                related: None,
                bytes: None,
                window_start: None,
            },
            AuditDetail::LiveView {
                activity: LiveViewActivity::Issue,
            },
            AuditDetail::Computer {
                activity: ComputerActivity::Create,
                stage: ComputerAuditStage::Reserved,
                task: None,
            },
        ];
        let drafts = details.into_iter().map(draft).collect::<Vec<_>>();
        assert_eq!(
            drafts
                .iter()
                .map(|draft| draft.detail().class())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            6
        );
        audit.a.append(&drafts).await.unwrap();
        let stored = audit
            .a
            .page(&scope, &AuditQuery::new(partition.clone()))
            .await
            .unwrap();
        assert_eq!(stored.records.len(), drafts.len());
        let sealer = AuditSealer::acquire(
            audit.a.clone(),
            Arc::new(AuditSigningKey::from_seed(&[7; 32])),
        )
        .await
        .unwrap();
        assert_eq!(seal_all(&sealer).await, drafts.len());
        let block = audit
            .b
            .blocks(&scope, &partition, None, 10)
            .await
            .unwrap()
            .remove(0);
        let unsealed = draft(AuditDetail::Read {
            method: AuditReadMethod::Status,
        });
        audit
            .b
            .append(std::slice::from_ref(&unsealed))
            .await
            .unwrap();
        let sealed_at = block.head.sealed_at;
        assert!(
            !sealer.retire(&block, sealed_at, &[]).await.unwrap(),
            "a block sealed at the cutoff is retained"
        );
        assert!(
            sealer
                .retire(&block, sealed_at + TimeDelta::seconds(1), &[])
                .await
                .unwrap()
        );
        let page = audit
            .b
            .page(&scope, &AuditQuery::new(partition.clone()))
            .await
            .unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].draft, unsealed);
        let range = audit.b.export_range(&scope, &partition).await.unwrap();
        assert!(range.first.is_none());
        assert_eq!(range.checkpoint, Some(block.checkpoint()));
        assert!(
            !sealer
                .retire(&block, sealed_at + TimeDelta::seconds(1), &[])
                .await
                .unwrap(),
            "a retired block cannot be retired again"
        );
        sealer.release().await.unwrap();
    })
    .await;
}
