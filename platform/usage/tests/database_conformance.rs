//! The usage queries and the `usage` schema against the server schema rules in
//! `docs/DATABASE.md`.
#[allow(dead_code, reason = "each test binary uses part of the shared fixture")]
#[path = "../../../testing/fixtures/src/store.rs"]
mod store;

#[allow(
    dead_code,
    reason = "each test binary uses part of the conformance API"
)]
#[path = "../../../testing/fixtures/src/database.rs"]
mod database;

use database::Conformance;

#[tokio::test]
async fn usage_queries_and_schema_follow_the_database_rules() {
    Conformance::new()
        .schema("usage")
        .queries_from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.sqlx"))
        .sources(concat!(env!("CARGO_MANIFEST_DIR"), "/queries"))
        .check()
        .await;
}
