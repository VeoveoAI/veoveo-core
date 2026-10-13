//! The store's queries and the kernel, identity and Gateway schemas against the
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

use database::{Conformance, Rule};

#[tokio::test]
async fn store_queries_and_kernel_schemas_follow_the_database_rules() {
    Conformance::new()
        .schema("kernel")
        .schema("identity")
        .schema("gateway")
        .queries_from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.sqlx"))
        .sources(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
        .all_tenants(
            "gateway.refresh_family",
            "a refresh family is addressed by its identifier or a token digest, and its \
             tenant travels in the tokens it issues",
        )
        .all_tenants(
            "gateway.control_revision",
            "a control-plane revision configures the whole installation; its tenant names \
             the one tenant a revision may belong to",
        )
        .pending(
            Rule::Schema,
            "gateway.task_route",
            "the owner and tenant foreign keys have no index, so deleting a principal or a \
             tenant scans the routes; no code path deletes either yet",
        )
        .check()
        .await;
}
