//! The Task runtime's queries and the `tasks` schema against the server schema rules in
//! `docs/DATABASE.md`.
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
async fn task_queries_and_schema_follow_the_database_rules() {
    Conformance::new()
        .schema("tasks")
        .queries_from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.sqlx"))
        .sources(concat!(env!("CARGO_MANIFEST_DIR"), "/queries"))
        .check()
        .await;
}
