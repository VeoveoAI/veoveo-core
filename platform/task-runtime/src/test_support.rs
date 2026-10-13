//! Owners, instances and Tasks for the unit tests that run on the PostgreSQL fixture.
use std::{collections::BTreeSet, sync::Arc};

use veoveo_platform_store::{Instance, PrincipalKind};
use veoveo_types::{
    AccessSubject, InvocationAuthority, InvocationProvenance, PolicyVersion, PrincipalId,
    ServerSlug, TaskId, TaskTypeName, TenantId, WorkContextId, WorkContextMembershipLevel,
    WorkContextOutputPolicy,
};

use crate::{CreateTask, RecoveryClass, TaskOwner, TaskRuntime, test_store::TestDb};

pub(crate) fn owner() -> TaskOwner {
    let principal = PrincipalId::parse("pilot").unwrap();
    let tenant = TenantId::parse("mission-tenant").unwrap();
    TaskOwner {
        principal_key: principal.clone(),
        principal_kind: PrincipalKind::User,
        issuer: "https://issuer.test".parse().unwrap(),
        subject: "pilot".parse().unwrap(),
        profile: "operator".parse().unwrap(),
        tenant_key: Some(tenant.clone()),
        data_labels: BTreeSet::new(),
        authority: InvocationAuthority {
            work_context: WorkContextId::parse("mission").unwrap(),
            tenant,
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

/// A process instance of `server` on the fixture's first pool.
pub(crate) async fn instance(db: &TestDb, server: &str) -> Arc<Instance> {
    let server = ServerSlug::parse(server).unwrap();
    Arc::new(Instance::start(&db.a, &server).await.unwrap())
}

/// A runtime of `server` on its own instance.
pub(crate) async fn runtime(db: &TestDb, server: &str) -> TaskRuntime {
    TaskRuntime::new(
        db.a.clone(),
        ServerSlug::parse(server).unwrap(),
        instance(db, server).await,
    )
}

/// A queued Task of the runtime's server, retained for `ttl_ms` once it finishes.
pub(crate) async fn create(runtime: &TaskRuntime, ttl_ms: Option<u64>) -> TaskId {
    runtime
        .create(CreateTask {
            task_id: TaskId::new(),
            owner: owner(),
            server: runtime.server().clone(),
            task_type: TaskTypeName::from_static("fixture.run"),
            request: serde_json::json!({}),
            recovery_class: RecoveryClass::Resume,
            idempotency_key: None,
            ttl_ms,
            poll_interval_ms: None,
            retention_pins: BTreeSet::new(),
        })
        .await
        .unwrap()
        .snapshot
        .task_id
}
