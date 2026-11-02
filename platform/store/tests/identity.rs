//! Identity records against a private PostgreSQL 18 cluster.
#[allow(dead_code, reason = "each test binary uses part of the shared fixture")]
#[path = "../../../testing/fixtures/src/store.rs"]
mod store;

use store::TestDb;
use veoveo_store::{
    PrincipalKind, StoreError, TenantId, deterministic_enterprise_id, deterministic_principal_id,
    deterministic_tenant_id,
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
