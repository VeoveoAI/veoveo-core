//! Audit stores over the PostgreSQL test fixture, as two Gateway replicas see them.
#![allow(dead_code, reason = "each test binary uses part of the shared support")]

#[path = "../../../../testing/fixtures/store.rs"]
pub mod store;

use std::{sync::Arc, time::Duration};
use veoveo_audit::{
    AuditActor, AuditBlock, AuditDetail, AuditDraft, AuditHealth, AuditHealthState, AuditOutcome,
    AuditPartition, AuditPrincipalKind, AuditReadScope, AuditReason, AuditRecord, AuditRequest,
    AuditStore, AuditTarget, AuditTargetRegistry,
    sealer::{AuditSealer, SealBacklog},
};

/// One fixture database and an audit store on each of its two owner pools.
pub struct AuditDb {
    pub db: store::TestDb,
    pub a: AuditStore,
    pub b: AuditStore,
}

impl AuditDb {
    pub async fn new() -> Self {
        Self::with_targets(Arc::new(AuditTargetRegistry::empty())).await
    }

    pub async fn with_targets(targets: Arc<AuditTargetRegistry>) -> Self {
        let db = store::TestDb::new().await;
        Self {
            a: AuditStore::new(db.a.clone(), targets.clone()),
            b: AuditStore::new(db.b.clone(), targets),
            db,
        }
    }
}

pub async fn within<T>(limit: Duration, future: impl Future<Output = T>) -> T {
    tokio::time::timeout(limit, future)
        .await
        .unwrap_or_else(|_| panic!("exceeded {limit:?}"))
}

pub async fn state(health: &mut AuditHealth, expected: AuditHealthState) {
    within(Duration::from_secs(20), async {
        while health.state() != expected {
            assert_ne!(
                health.state(),
                AuditHealthState::Failed,
                "audit service failed"
            );
            health.changed().await.unwrap();
        }
    })
    .await;
}

/// An installation record of a background operation.
pub fn draft(detail: AuditDetail) -> AuditDraft {
    AuditDraft::builder(
        AuditRequest::background(),
        AuditTarget::Installation,
        detail,
        AuditOutcome::Allowed,
        AuditReason::Accepted,
    )
    .build()
    .unwrap()
}

/// Seal until every committed record is sealed; return how many records were sealed.
pub async fn seal_all(sealer: &AuditSealer) -> usize {
    within(Duration::from_secs(30), async {
        let mut sealed = 0;
        loop {
            let pass = sealer.seal_pending().await.unwrap();
            sealed += pass.sealed;
            match pass.backlog {
                SealBacklog::Empty => return sealed,
                SealBacklog::Sealable => {}
                SealBacklog::Blocked => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
    })
    .await
}

/// Every retained block of a partition with its records, in chain order.
pub async fn archive(
    store: &AuditStore,
    scope: &AuditReadScope,
    partition: &AuditPartition,
) -> Vec<(AuditBlock, Vec<AuditRecord>)> {
    let mut after = None;
    let mut blocks = Vec::new();
    loop {
        let page = store.blocks(scope, partition, after, 16).await.unwrap();
        if page.is_empty() {
            return blocks;
        }
        for block in page {
            let records = store.block_records(scope, &block).await.unwrap();
            after = Some(block.head.sequence);
            blocks.push((block, records));
        }
        assert!(
            blocks.len() <= 256,
            "fixture archive exceeded its page budget"
        );
    }
}

pub fn user(tenant: &str, name: &str) -> AuditActor {
    AuditActor {
        principal: name.parse().unwrap(),
        kind: AuditPrincipalKind::User,
        tenant: Some(tenant.parse().unwrap()),
        oauth_client: None,
        session_family: None,
        delegating_principal: None,
        managed_agent: None,
    }
}
