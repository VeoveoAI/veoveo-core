//! Audit stores over the PostgreSQL test fixture, as two replicas see them.
#![allow(dead_code, reason = "each test binary uses part of the shared support")]

#[path = "../../../../testing/fixtures/store.rs"]
pub mod store;

#[path = "../../../../testing/fixtures/audit_owner.rs"]
pub mod audit_owner;

use std::{
    sync::{Arc, LazyLock},
    time::Duration,
};
use veoveo_audit::{
    AuditActor, AuditBlock, AuditDetail, AuditDraft, AuditHealth, AuditHealthState, AuditOutcome,
    AuditPartition, AuditPrincipalKind, AuditReadScope, AuditRecord, AuditRegistry, AuditRequest,
    AuditStore, platform,
    sealer::{AuditSealer, SealBacklog},
};

/// The platform's classes and the fixture owner's `fixture.note`.
pub fn registry() -> AuditRegistry {
    let mut builder = AuditRegistry::builder();
    audit_owner::declare_audit_detail(&mut builder).unwrap();
    builder.build()
}

static DETAILS: LazyLock<AuditRegistry> = LazyLock::new(registry);

/// A `fixture.note` detail; distinct counts give distinct details.
pub fn note(count: u32) -> AuditDetail {
    DETAILS.detail(&audit_owner::Note { count }).unwrap()
}

/// The platform's `task.update` detail.
pub fn task_update() -> AuditDetail {
    DETAILS.detail(&platform::TaskUpdate {}).unwrap()
}

/// The platform's `task.cancel` detail.
pub fn task_cancel() -> AuditDetail {
    DETAILS.detail(&platform::TaskCancel {}).unwrap()
}

/// One fixture database and an audit store on each of its two owner pools.
pub struct AuditDb {
    pub db: store::TestDb,
    pub a: AuditStore,
    pub b: AuditStore,
}

impl AuditDb {
    pub async fn new() -> Self {
        Self::with_registry(Arc::new(registry())).await
    }

    /// Stores over `registry`, whose declarations are already recorded.
    pub async fn with_registry(registry: Arc<AuditRegistry>) -> Self {
        let db = store::TestDb::new().await;
        let audit = Self {
            a: AuditStore::new(db.a.clone(), registry.clone()),
            b: AuditStore::new(db.b.clone(), registry),
            db,
        };
        audit.a.declare().await.unwrap();
        audit
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
    AuditDraft::builder(AuditRequest::background(), detail, AuditOutcome::Success)
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
        work_context: None,
    }
}
