//! Owner targets through filters, transactions, change streams, sealing and export lines,
//! admitted by one immutable registry.
mod support;

#[path = "../../../testing/fixtures/computer_target.rs"]
mod computer_target;

use computer_target::{ComputerAuditTarget, ComputerId, register_audit_target};
use futures::StreamExt;
use std::{sync::Arc, time::Duration};
use support::{AuditDb, seal_all, within};
use veoveo_audit::{
    integrity::{AuditSigningKey, record_root},
    sealer::AuditSealer,
    *,
};

/// Another owner whose payload carries the same UUID as a Computer target.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(from = "ShadowWire", into = "ShadowWire")]
struct ShadowTarget {
    lookup_id: uuid::Uuid,
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum ShadowWire {
    #[serde(rename = "shadow_computer")]
    Shadow { lookup_id: uuid::Uuid },
}
impl From<ShadowWire> for ShadowTarget {
    fn from(wire: ShadowWire) -> Self {
        let ShadowWire::Shadow { lookup_id } = wire;
        Self { lookup_id }
    }
}
impl From<ShadowTarget> for ShadowWire {
    fn from(target: ShadowTarget) -> Self {
        Self::Shadow {
            lookup_id: target.lookup_id,
        }
    }
}
impl schemars::JsonSchema for ShadowTarget {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ShadowTarget".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"type":"object","additionalProperties":false,"properties":{"kind":{"type":"string","const":"shadow_computer"},"lookup_id":{"type":"string","format":"uuid"}},"required":["kind","lookup_id"]})
    }
}
impl AuditTargetOwner for ShadowTarget {
    const KIND: &'static str = "shadow_computer";
}

fn targets() -> Arc<AuditTargetRegistry> {
    let mut builder = AuditTargetRegistry::builder();
    register_audit_target(&mut builder).unwrap();
    builder.register::<ShadowTarget>().unwrap();
    Arc::new(builder.build())
}

fn draft(target: AuditTarget) -> AuditDraft {
    AuditDraft::builder(
        AuditRequest::background(),
        target,
        AuditDetail::Computer {
            activity: ComputerActivity::Create,
            stage: ComputerAuditStage::Reserved,
            task: None,
        },
        AuditOutcome::Succeeded,
        AuditReason::Accepted,
    )
    .build()
    .unwrap()
}

#[tokio::test]
async fn owner_targets_keep_exact_filters_transactions_streams_sealing_and_export_lines() {
    within(Duration::from_secs(90), async {
        let registry = targets();
        let audit = AuditDb::with_targets(registry.clone()).await;
        let selected = ComputerId::new();
        let target = registry
            .target(ComputerAuditTarget { computer: selected })
            .unwrap();
        let records = vec![
            draft(target.clone()),
            draft(
                registry
                    .target(ShadowTarget {
                        lookup_id: selected.as_uuid(),
                    })
                    .unwrap(),
            ),
            draft(
                registry
                    .target(ComputerAuditTarget {
                        computer: ComputerId::new(),
                    })
                    .unwrap(),
            ),
            draft(target.clone()),
        ];
        let scope = AuditReadScope::new(None, true);
        let partition = AuditPartition::Installation;
        let mut live = audit.b.live(&scope, &partition).await.unwrap();
        audit.a.append(&records).await.unwrap();
        let notice = within(Duration::from_secs(10), async {
            loop {
                match live.next().await.unwrap().unwrap() {
                    AuditLiveChange::Record(record) => break record,
                    AuditLiveChange::Reconcile => {}
                    AuditLiveChange::RetainedRangeChanged => panic!("nothing was retired"),
                }
            }
        })
        .await;
        assert!(
            records
                .iter()
                .any(|record| record.id() == notice.draft.id())
        );
        drop(live);

        let mut query = AuditQuery::new(partition.clone());
        query.target = Some(target.clone());
        query.limit = 1;
        let first = audit.b.page(&scope, &query).await.unwrap();
        assert_eq!(first.records.len(), 1);
        assert_eq!(first.records[0].draft.target(), &target);
        assert!(first.next.is_some());
        let reconnected = AuditStore::new(audit.db.connect().await, registry.clone());
        assert!(std::ptr::eq(audit.a.targets(), reconnected.targets()));
        let recovered = reconnected.page(&scope, &query).await.unwrap();
        assert_eq!(recovered.records[0].draft.target(), &target);
        query.cursor = first.next.clone();
        let second = audit.b.page(&scope, &query).await.unwrap();
        assert_eq!(second.records.len(), 1);
        assert_eq!(second.records[0].draft.target(), &target);
        assert_ne!(second.records[0].draft.id(), first.records[0].draft.id());
        assert!(second.next.is_none());

        let alien = targets()
            .target(ComputerAuditTarget { computer: selected })
            .unwrap();
        query.cursor = None;
        query.target = Some(alien.clone());
        assert!(matches!(
            audit.a.page(&scope, &query).await,
            Err(AuditStoreError::Target(AuditTargetError::Registry))
        ));
        assert!(matches!(
            audit.a.append(&[draft(alien.clone())]).await,
            Err(AuditStoreError::Target(AuditTargetError::Registry))
        ));
        assert!(matches!(
            AuditTransactionWrite::new(audit.a.targets(), draft(alien)),
            Err(AuditAppendError::Target(AuditTargetError::Registry))
        ));

        sqlx::query(include_str!("queries/append/domain_table.sql"))
            .execute(audit.db.a.pool())
            .await
            .unwrap();
        let conflict = AuditDraft::builder(
            records[0].request().clone(),
            target.clone(),
            records[0].detail().clone(),
            AuditOutcome::Failed,
            AuditReason::Unavailable,
        )
        .identity(records[0].id())
        .occurred_at(records[0].occurred_at())
        .build()
        .unwrap();
        let mut transaction = audit.db.a.pool().begin().await.unwrap();
        sqlx::query(include_str!("queries/append/domain_insert.sql"))
            .bind(1)
            .execute(&mut *transaction)
            .await
            .unwrap();
        assert!(
            AuditTransactionWrite::batch(audit.a.targets(), [draft(target.clone()), conflict])
                .unwrap()
                .append(&mut transaction)
                .await
                .is_err()
        );
        transaction.rollback().await.unwrap();
        let domain_rows =
            sqlx::query_scalar::<_, i64>(include_str!("queries/append/domain_count.sql"))
                .fetch_one(audit.db.b.pool())
                .await
                .unwrap();
        assert_eq!(
            domain_rows, 0,
            "the domain write rolls back with the audit append"
        );

        let mut all = AuditQuery::new(partition.clone());
        let stored = audit.b.page(&scope, &all).await.unwrap();
        assert_eq!(stored.records.len(), 4);
        let sealer = AuditSealer::acquire(
            audit.a.clone(),
            Arc::new(AuditSigningKey::from_seed(&[71; 32])),
        )
        .await
        .unwrap();
        assert_eq!(seal_all(&sealer).await, 4);
        sealer.release().await.unwrap();
        let block = audit
            .b
            .blocks(&scope, &partition, None, 1)
            .await
            .unwrap()
            .remove(0);
        let exported = audit.b.block_records(&scope, &block).await.unwrap();
        assert_eq!(record_root(&exported).unwrap(), block.head.root);
        all.target = Some(target);
        let filtered = audit
            .b
            .filtered_block_records(&scope, &block, &all)
            .await
            .unwrap();
        assert_eq!(filtered.len(), 2);
        for record in exported {
            let line = AuditExportLine::Record {
                record: Box::new(record),
            };
            let bytes = serde_json::to_string(&line).unwrap();
            assert!(matches!(
                audit
                    .b
                    .targets()
                    .decoder()
                    .from_str::<AuditExportLine>(&bytes)
                    .unwrap(),
                AuditExportLine::Record { .. }
            ));
        }

        // Rewrite the page's lookahead row with a target no codec admits.
        sqlx::query_file!(
            "tests/queries/targets/unbind_target.sql",
            stored.records[1].draft.id() as AuditRecordId
        )
        .execute(audit.db.a.pool())
        .await
        .unwrap();
        let mut lookahead = AuditQuery::new(partition);
        lookahead.limit = 1;
        assert!(matches!(
            audit.b.page(&scope, &lookahead).await,
            Err(AuditStoreError::Target(AuditTargetError::Unbound(_)))
        ));
    })
    .await;
}
