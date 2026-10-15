//! Identity records against a private PostgreSQL 18 cluster.
#[allow(dead_code, reason = "each test binary uses part of the shared fixture")]
#[path = "../../../testing/fixtures/store.rs"]
mod store;

use store::TestDb;
use veoveo_store::{
    GrantPermission, GrantSubjectKind, PrincipalKind, StoreError, TenantId,
    WorkContextInitialGrantRecord, WorkContextMembershipLevel, WorkContextMembershipRuleRecord,
    deterministic_enterprise_id, deterministic_principal_id, deterministic_tenant_id,
    deterministic_work_context_id,
};
use veoveo_types::ServerSlug;

const ISSUER: &str = "https://issuer.example";

#[tokio::test]
async fn identities_are_recorded_once_and_keep_their_security_fields() {
    let db = TestDb::new().await;
    let identity =
        db.a.ensure_identity(
            "acme",
            "alice",
            ISSUER,
            "https://issuer.example/users/alice",
            PrincipalKind::User,
        )
        .await
        .unwrap();
    assert_eq!(identity.tenant_id, deterministic_tenant_id("acme").unwrap());
    assert_eq!(
        identity.principal_id,
        deterministic_principal_id("acme", "alice").unwrap()
    );
    let stored = |id| {
        let pool = db.a.pool().clone();
        async move {
            sqlx::query_file!("tests/queries/identity/principal.sql", id as _)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    let row = stored(identity.principal_id).await;
    assert_eq!(row.tenant, identity.tenant_id);
    assert_eq!(row.kind, PrincipalKind::User);
    assert_eq!(row.principal_key, "alice");
    assert_eq!(row.display_name, "alice");

    db.b.ensure_named_identity(
        "acme",
        "alice",
        ISSUER,
        "https://issuer.example/users/alice",
        PrincipalKind::User,
        "Alice Example",
    )
    .await
    .unwrap();
    db.a.ensure_identity(
        "acme",
        "alice",
        ISSUER,
        "https://issuer.example/users/alice",
        PrincipalKind::User,
    )
    .await
    .unwrap();
    assert_eq!(
        stored(identity.principal_id).await.display_name,
        "Alice Example"
    );

    let conflict =
        db.a.ensure_identity(
            "acme",
            "alice",
            "https://attacker.example",
            "https://issuer.example/users/alice",
            PrincipalKind::User,
        )
        .await;
    assert!(matches!(
        conflict,
        Err(StoreError::IdentityConflict {
            entity: "principal",
            ..
        })
    ));
    let duplicate_subject =
        db.a.ensure_identity(
            "acme",
            "alice-again",
            ISSUER,
            "https://issuer.example/users/alice",
            PrincipalKind::User,
        )
        .await;
    assert!(matches!(
        duplicate_subject,
        Err(StoreError::IdentityConflict {
            entity: "principal",
            ..
        })
    ));
    assert!(matches!(
        db.a.ensure_identity("acme", " ", ISSUER, "bob", PrincipalKind::User)
            .await,
        Err(StoreError::InvalidIdentityField {
            field: "principal_key",
            ..
        })
    ));
}

#[tokio::test]
async fn a_tenant_stored_under_another_key_conflicts() {
    let db = TestDb::new().await;
    db.a.ensure_identity("acme", "alice", ISSUER, "alice", PrincipalKind::User)
        .await
        .unwrap();
    sqlx::query_file!(
        "tests/queries/identity/insert_tenant.sql",
        deterministic_tenant_id("beta").unwrap() as TenantId,
        deterministic_enterprise_id() as _,
        "not-beta",
    )
    .execute(db.a.pool())
    .await
    .unwrap();
    assert!(matches!(
        db.a.ensure_identity("beta", "bob", ISSUER, "bob", PrincipalKind::Service)
            .await,
        Err(StoreError::IdentityConflict {
            entity: "tenant",
            ..
        })
    ));
}

#[tokio::test]
async fn concurrent_first_logins_converge_on_one_identity() {
    let db = TestDb::new().await;
    let server = db.server_store(&ServerSlug::parse("clock").unwrap()).await;
    let stores = [db.a.clone(), db.b.clone(), server];
    for round in 0..24 {
        // Every round names a tenant and principals nobody recorded yet, so the calls
        // race to create the enterprise, the tenant and each principal.
        let tenant = format!("tenant-{round}");
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(12));
        let calls = (0..12).map(|call| {
            let store = stores[call % stores.len()].clone();
            let barrier = barrier.clone();
            let tenant = tenant.clone();
            let principal = format!("user-{}", call % 3);
            async move {
                barrier.wait().await;
                store
                    .ensure_identity(
                        &tenant,
                        &principal,
                        ISSUER,
                        &format!("https://issuer.example/{tenant}/{principal}"),
                        PrincipalKind::User,
                    )
                    .await
            }
        });
        for result in futures::future::join_all(calls).await {
            result.unwrap();
        }
        for principal in ["user-0", "user-1", "user-2"] {
            let count = sqlx::query_file_scalar!(
                "tests/queries/identity/principal_count.sql",
                &tenant,
                principal,
            )
            .fetch_one(db.a.pool())
            .await
            .unwrap();
            assert_eq!(count, 1, "{tenant} {principal}");
        }
    }
}

#[tokio::test]
async fn registered_servers_record_identities_through_the_kernel_function() {
    let db = TestDb::new().await;
    let server = db.server_store(&ServerSlug::parse("clock").unwrap()).await;
    let identity = server
        .ensure_identity(
            "acme",
            "clock-service",
            ISSUER,
            "clock",
            PrincipalKind::Service,
        )
        .await
        .unwrap();
    let row = sqlx::query_file!(
        "tests/queries/identity/principal.sql",
        identity.principal_id as _
    )
    .fetch_one(db.a.pool())
    .await
    .unwrap();
    assert_eq!(row.kind, PrincipalKind::Service);
    assert!(
        sqlx::query_file!(
            "tests/queries/identity/principal.sql",
            identity.principal_id as _
        )
        .fetch_one(server.pool())
        .await
        .is_err(),
        "servers read no identity table"
    );
}

#[tokio::test]
async fn work_contexts_read_with_their_grants_and_memberships_in_order() {
    let db = TestDb::new().await;
    let identity =
        db.a.ensure_identity("acme", "alice", ISSUER, "alice", PrincipalKind::User)
            .await
            .unwrap();
    let id = deterministic_work_context_id("acme", "development").unwrap();
    sqlx::query_file!(
        "tests/queries/identity/insert_work_context.sql",
        id as _,
        identity.tenant_id as _,
        "development",
        "Development",
        "7",
        GrantSubjectKind::Group as _,
        "builders",
        Some("internal"),
        &["pii".to_owned()][..],
    )
    .execute(db.a.pool())
    .await
    .unwrap();
    for (ordinal, kind, key, permission) in [
        (
            0,
            GrantSubjectKind::Principal,
            "alice",
            GrantPermission::Admin,
        ),
        (1, GrantSubjectKind::Group, "readers", GrantPermission::Read),
    ] {
        sqlx::query_file!(
            "tests/queries/identity/insert_work_context_grant.sql",
            id as _,
            ordinal,
            kind as _,
            key,
            permission as _,
        )
        .execute(db.a.pool())
        .await
        .unwrap();
    }
    for (ordinal, level, principals, clients) in [
        (
            0,
            WorkContextMembershipLevel::Owner,
            vec!["alice".to_owned()],
            Vec::new(),
        ),
        (
            1,
            WorkContextMembershipLevel::Viewer,
            vec!["alice".to_owned(), "bob".to_owned()],
            vec!["console".to_owned()],
        ),
    ] {
        let groups: Vec<String> = Vec::new();
        sqlx::query_file!(
            "tests/queries/identity/insert_work_context_membership.sql",
            id as _,
            ordinal,
            level as _,
            &principals[..],
            &groups[..],
            &["member".to_owned()][..],
            &clients[..],
        )
        .execute(db.a.pool())
        .await
        .unwrap();
    }

    let context =
        db.b.work_context_by_key(identity.tenant_id, "development")
            .await
            .unwrap()
            .unwrap();
    assert_eq!(context.id, id);
    assert_eq!(context.policy_revision, "7");
    assert_eq!(context.output_policy.owner_kind, GrantSubjectKind::Group);
    assert_eq!(
        context.output_policy.classification.as_deref(),
        Some("internal")
    );
    assert_eq!(context.output_policy.data_labels, vec!["pii"]);
    assert_eq!(
        context.output_policy.initial_grants,
        vec![
            WorkContextInitialGrantRecord {
                subject_kind: GrantSubjectKind::Principal,
                subject_key: "alice".into(),
                permission: GrantPermission::Admin,
            },
            WorkContextInitialGrantRecord {
                subject_kind: GrantSubjectKind::Group,
                subject_key: "readers".into(),
                permission: GrantPermission::Read,
            },
        ]
    );
    assert_eq!(
        context.memberships[1],
        WorkContextMembershipRuleRecord {
            level: WorkContextMembershipLevel::Viewer,
            principals: vec!["alice".into(), "bob".into()],
            groups: Vec::new(),
            roles: vec!["member".into()],
            oauth_clients: vec!["console".into()],
        }
    );
    assert_eq!(
        context.membership_for_principal("alice"),
        Some(WorkContextMembershipLevel::Owner)
    );
    assert_eq!(
        context.membership_for_oauth_client("console"),
        Some(WorkContextMembershipLevel::Viewer)
    );
    assert!(
        db.a.work_context_by_key(identity.tenant_id, "missing")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn domain_types_bind_through_their_wire_forms_and_reject_unknown_values() {
    let db = TestDb::new().await;
    let row = sqlx::query_file!("tests/queries/identity/decode_kind.sql", "service")
        .fetch_one(db.a.pool())
        .await
        .unwrap();
    assert_eq!(row.kind, PrincipalKind::Service);
    assert_eq!(row.kinds, vec![PrincipalKind::Service]);
    let unknown = sqlx::query_file!("tests/queries/identity/decode_kind.sql", "robot")
        .fetch_one(db.a.pool())
        .await;
    assert!(matches!(unknown, Err(sqlx::Error::ColumnDecode { .. })));

    let tenant = deterministic_tenant_id("acme").unwrap();
    let row = sqlx::query_file!(
        "tests/queries/identity/decode_tenant.sql",
        tenant as TenantId
    )
    .fetch_one(db.a.pool())
    .await
    .unwrap();
    assert_eq!(row.tenant, tenant);
    assert_eq!(row.tenants, vec![tenant]);
}
