//! The store's queries and the kernel and identity schemas against the
//! server schema rules in `docs/DATABASE.md`.
#[allow(dead_code, reason = "each test binary uses part of the shared fixture")]
#[path = "../../../testing/fixtures/store.rs"]
mod store;

#[allow(
    dead_code,
    reason = "each test binary uses part of the conformance API"
)]
#[path = "../../../testing/fixtures/database.rs"]
mod database;

use database::Conformance;

#[tokio::test]
async fn store_queries_and_kernel_schemas_follow_the_database_rules() {
    Conformance::new()
        .schema("kernel")
        .schema("identity")
        .queries_from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.sqlx"))
        .sources(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
        .check()
        .await;
}
