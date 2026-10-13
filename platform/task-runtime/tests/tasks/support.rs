//! Fixtures shared by the Task runtime tests.
use std::{collections::BTreeSet, future::Future, sync::Arc, time::Duration};

use veoveo_platform_store::PlatformStore;
use veoveo_task_runtime::{
    CreateTask, Instance, InstanceId, PrincipalKind, RecoveryClass, TaskOwner, TaskRuntime,
    TaskSnapshot,
};
use veoveo_types::{
    AccessSubject, DataLabelId, InvocationAuthority, InvocationProvenance, PolicyVersion,
    PrincipalId, ServerSlug, TaskId, TaskTypeName, TenantId, WorkContextId,
    WorkContextMembershipLevel, WorkContextOutputPolicy,
};

use crate::store::TestDb;

pub const SERVER: &str = "time";
pub const TASK_TYPE: TaskTypeName = TaskTypeName::from_static("time.expand_schedule");

/// Run one test case within its deadline.
pub async fn within<T>(case: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(60), case)
        .await
        .expect("test case exceeded 60 seconds")
}

pub fn slug(server: &str) -> ServerSlug {
    ServerSlug::parse(server).unwrap()
}

/// A process instance of `server` on `store`.
pub async fn instance(store: &PlatformStore, server: &str) -> Arc<Instance> {
    Arc::new(
        Instance::start(store, &slug(server))
            .await
            .expect("start instance"),
    )
}

/// A runtime of `server` on its own process instance.
pub async fn runtime_for(store: &PlatformStore, server: &str) -> TaskRuntime {
    TaskRuntime::new(store.clone(), slug(server), instance(store, server).await)
}

/// A runtime of the default server, as one installation-owner replica.
pub async fn runtime(db: &TestDb) -> TaskRuntime {
    runtime_for(&db.a, SERVER).await
}

/// End the process behind `runtime`'s instance without a clean shutdown, as a crash
/// would, then reap it from another connection.
pub async fn kill_and_reap(db: &TestDb, runtime: &TaskRuntime) {
    let id = runtime.instance().id();
    let pid = sqlx::query_file_scalar!("tests/queries/instance_backend.sql", id as InstanceId)
        .fetch_one(db.b.pool())
        .await
        .expect("find the instance's lock session");
    terminate(db, pid).await;
    while db.b.instance_alive(id).await.expect("check the instance") {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Another instance's periodic reap may have deleted the row already.
    db.b.reap_instances().await.expect("reap");
}

/// Close every notification listener's connection; each listener reconnects.
pub async fn interrupt_listeners(db: &TestDb) {
    let pids = sqlx::query_file_scalar!("tests/queries/listener_backends.sql")
        .fetch_all(db.b.pool())
        .await
        .expect("find listener sessions");
    assert!(!pids.is_empty(), "a listener is connected");
    for pid in pids {
        terminate(db, pid).await;
    }
}

async fn terminate(db: &TestDb, pid: i32) {
    let terminated = sqlx::query_file_scalar!("tests/queries/terminate_backend.sql", pid)
        .fetch_one(db.b.pool())
        .await
        .expect("terminate a session");
    assert!(terminated);
}

pub struct OwnerSpec<'a> {
    pub principal: &'a str,
    pub surface: &'a str,
    pub tenant: Option<&'a str>,
    pub labels: &'a [&'a str],
    pub work_context: &'a str,
}

impl Default for OwnerSpec<'_> {
    fn default() -> Self {
        Self {
            principal: "pilot",
            surface: "operator",
            tenant: Some("mission-tenant"),
            labels: &[],
            work_context: "mission",
        }
    }
}

pub fn owner_with(spec: OwnerSpec<'_>) -> TaskOwner {
    let principal = PrincipalId::parse(spec.principal).unwrap();
    let tenant = spec.tenant.map(|tenant| TenantId::parse(tenant).unwrap());
    TaskOwner {
        principal_key: principal.clone(),
        principal_kind: PrincipalKind::User,
        issuer: "https://issuer.test".parse().unwrap(),
        subject: spec.principal.parse().unwrap(),
        surface: spec.surface.parse().unwrap(),
        tenant_key: tenant.clone(),
        data_labels: spec
            .labels
            .iter()
            .map(|label| DataLabelId::parse(label).unwrap())
            .collect(),
        authority: InvocationAuthority {
            work_context: WorkContextId::parse(spec.work_context).unwrap(),
            tenant: tenant.unwrap_or_else(|| TenantId::parse("installation").unwrap()),
            membership: WorkContextMembershipLevel::Owner,
            policy_revision: PolicyVersion::parse("r1").unwrap(),
            output_policy: WorkContextOutputPolicy {
                owner: AccessSubject::Principal(principal.clone()),
                initial_grants: Vec::new(),
                classification: None,
                data_labels: BTreeSet::new(),
            },
            provenance: InvocationProvenance::Direct {
                initiator: principal,
            },
        },
    }
}

pub fn owner() -> TaskOwner {
    owner_with(OwnerSpec::default())
}

pub fn draft(
    runtime: &TaskRuntime,
    owner: &TaskOwner,
    recovery_class: RecoveryClass,
) -> CreateTask {
    CreateTask {
        task_id: TaskId::new(),
        owner: owner.clone(),
        server: runtime.server().clone(),
        task_type: TASK_TYPE,
        request: serde_json::json!({"kind": "expand_schedule", "request": {"days": 3}}),
        recovery_class,
        idempotency_key: None,
        ttl_ms: None,
        poll_interval_ms: Some(1_000),
        retention_pins: BTreeSet::new(),
    }
}

pub async fn create(runtime: &TaskRuntime, recovery_class: RecoveryClass) -> TaskSnapshot {
    runtime
        .create(draft(runtime, &owner(), recovery_class))
        .await
        .expect("create Task")
        .snapshot
}

/// A Task this runtime claimed.
pub async fn claimed(runtime: &TaskRuntime, recovery_class: RecoveryClass) -> TaskSnapshot {
    let created = create(runtime, recovery_class).await;
    runtime.claim(created.task_id).await.expect("claim Task")
}
