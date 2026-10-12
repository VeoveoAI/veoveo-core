//! The PostgreSQL test fixture: isolated databases, service credentials and cleanup.
#[allow(dead_code, reason = "each test binary uses part of the shared fixture")]
#[path = "../../../testing/fixtures/store.rs"]
mod store;

use store::TestDb;
use veoveo_platform_store::{PlatformStore, PrincipalKind, StoreConfig, deterministic_tenant_id};

#[tokio::test]
async fn each_fixture_has_its_own_database_and_drops_it() {
    let first = TestDb::new().await;
    let second = TestDb::new().await;
    assert_ne!(first.database(), second.database());
    first
        .a
        .ensure_identity(
            "acme",
            "alice",
            "https://issuer.example",
            "alice",
            PrincipalKind::User,
        )
        .await
        .unwrap();
    let tenant = deterministic_tenant_id("acme").unwrap();
    assert!(
        second
            .a
            .work_context_by_key(tenant, "development")
            .await
            .unwrap()
            .is_none()
    );
    let name = first.database().to_owned();
    drop(first);
    let exists = sqlx::query_file_scalar!("tests/queries/fixture/database_exists.sql", name)
        .fetch_one(second.a.pool())
        .await
        .unwrap();
    assert!(!exists);
}

#[tokio::test]
async fn services_connect_with_the_url_and_password_the_fixture_reports() {
    let db = TestDb::new().await;
    assert!(
        !db.url()
            .contains(secrecy::ExposeSecret::expose_secret(db.password()))
    );
    let config = StoreConfig::builder(db.url(), db.password().clone())
        .build()
        .unwrap();
    let service = PlatformStore::connect(config).await.unwrap();
    service.migrate().await.unwrap();
    service.close().await;
    let third = db.connect().await;
    assert!(third.current_servers().await.unwrap().is_empty());
}
