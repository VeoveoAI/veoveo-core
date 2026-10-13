//! The audit log's queries and the `audit` schema against the server schema rules in
//! `docs/DATABASE.md`. Audit partitions take the place of tenants.
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

/// The tables whose rows each belong to one audit partition.
const PARTITIONED: [&str; 8] = [
    "audit.record",
    "audit.daily",
    "audit.block",
    "audit.partition_head",
    "audit.retention_anchor",
    "audit.export_delivery",
    "audit.export_cursor",
    "audit.indexing_receipt",
];

#[tokio::test]
async fn audit_queries_and_schema_follow_the_database_rules() {
    let mut check = Conformance::new()
        .schema("audit")
        .queries_from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.sqlx"))
        .sources(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
        .sources(concat!(env!("CARGO_MANIFEST_DIR"), "/contract/src"));
    for table in PARTITIONED {
        check = check.tenant_columns(table, ["partition"]);
    }
    check
        .pending(
            Rule::IndexUse,
            "platform/audit/src/store/seal/pending.sql",
            "the plan reads record_commit_order from its start and compares each record \
             with the sealed position in a join filter; a scalar subquery for the sealed \
             position would make it an index condition",
        )
        .pending(
            Rule::IndexUse,
            "platform/audit/src/store/delivery/retention_candidates.sql",
            "the acknowledgement test hashes every delivered export_delivery row; a \
             per-block test of (partition, sequence, destination = ANY) would seek the \
             primary key",
        )
        .check()
        .await;
}
