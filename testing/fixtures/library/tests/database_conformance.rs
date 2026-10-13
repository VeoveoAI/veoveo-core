//! The database conformance checks against fixture servers: conforming queries pass,
//! each broken rule is reported under its query or table, and offline query metadata
//! is attributed to its `.sql` files.
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use veoveo_test_fixtures::database::{Conformance, Report, Rule};
use veoveo_types::ServerSlug;

fn slug(value: &str) -> ServerSlug {
    ServerSlug::parse(value).unwrap()
}

async fn migrator(name: &str) -> Migrator {
    Migrator::new(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/conformance")
            .join(name),
    )
    .await
    .unwrap()
}

fn ledger(migrator: Migrator) -> Conformance {
    Conformance::new().server(&slug("ledger"), migrator)
}

/// The violations of `subject`, as `(rule, message)`.
fn of<'r>(report: &'r Report, subject: &str) -> Vec<(Rule, &'r str)> {
    report
        .violations
        .iter()
        .filter(|violation| violation.subject == subject)
        .map(|violation| (violation.rule, violation.message.as_str()))
        .collect()
}

fn assert_reports(report: &Report, subject: &str, rule: Rule, fragment: &str) {
    assert!(
        of(report, subject)
            .iter()
            .any(|(found, message)| *found == rule && message.contains(fragment)),
        "expected {rule:?} containing {fragment:?} for {subject}\n{report}"
    );
}

const CONFORMING: &[(&str, &str)] = &[
    (
        "account by name",
        "SELECT id, created_at FROM ledger.account WHERE tenant = $1 AND name = $2",
    ),
    (
        "account page",
        "SELECT id, name FROM ledger.account WHERE tenant = $1 AND (created_at, id) > ($2, $3) \
         ORDER BY created_at, id LIMIT $4",
    ),
    (
        "entries of an account",
        "SELECT e.id, e.amount FROM ledger.entry AS e \
         JOIN ledger.account AS a ON a.id = e.account AND a.tenant = e.tenant \
         WHERE a.tenant = $1 AND a.id = $2 ORDER BY e.recorded_at LIMIT $3",
    ),
    (
        "open account",
        "INSERT INTO ledger.account (id, tenant, name, created_at) VALUES ($1, $2, $3, $4)",
    ),
    (
        "rename account",
        "UPDATE ledger.account SET name = $3 WHERE tenant = $1 AND id = $2",
    ),
    (
        "prune entries",
        "DELETE FROM ledger.entry WHERE id IN (SELECT id FROM ledger.entry \
         WHERE tenant = $1 AND expires_at <= $2 ORDER BY expires_at LIMIT $3 \
         FOR UPDATE SKIP LOCKED)",
    ),
    (
        "balance",
        "SELECT coalesce(sum(amount), 0)::bigint FROM ledger.entry \
         WHERE tenant = $1 AND account = $2",
    ),
    (
        "annotate a locked entry",
        "WITH locked AS (SELECT id FROM ledger.entry WHERE tenant = $1 AND id = $2 FOR UPDATE) \
         UPDATE ledger.entry SET note = $3 WHERE id = (SELECT id FROM locked)",
    ),
    ("setting", "SELECT value FROM ledger.setting"),
    ("set setting", "UPDATE ledger.setting SET value = $1"),
    (
        "entries by identity",
        "SELECT id FROM ledger.entry WHERE tenant = $1 AND id = ANY ($2)",
    ),
    (
        "tenant from a parameter row",
        "SELECT a.id FROM (SELECT $1::uuid AS tenant) AS p \
         JOIN ledger.account AS a ON a.tenant = p.tenant AND a.name = $2",
    ),
    (
        "sweep",
        "-- veoveo: all tenants (the sweep removes every tenant's expired entries)\n\
         DELETE FROM ledger.entry WHERE id IN (SELECT id FROM ledger.entry \
         WHERE expires_at <= $1 ORDER BY expires_at LIMIT $2)",
    ),
    (
        "export after a position read once",
        "-- veoveo: all tenants (the exporter reads every tenant's accounts)\n\
         SELECT a.id FROM ledger.account AS a \
         WHERE (a.created_at, a.id) > (SELECT p.created_at, p.id FROM ledger.export_position AS p) \
         ORDER BY a.created_at, a.id LIMIT $1",
    ),
];

#[tokio::test]
async fn conforming_queries_pass_and_report_their_exemptions() {
    let mut check = ledger(migrator("ledger").await);
    for (label, sql) in CONFORMING {
        check = check.query(*label, *sql);
    }
    let report = check.run().await;
    for (label, _) in CONFORMING {
        assert_eq!(of(&report, label), vec![], "{label}\n{report}");
    }
    assert!(
        report
            .exemptions
            .iter()
            .any(|exemption| exemption.subject == "sweep"
                && exemption.rule == Rule::TenantPredicate
                && exemption.reason == "the sweep removes every tenant's expired entries"),
        "{report}"
    );
}

#[tokio::test]
async fn each_broken_query_rule_is_reported_under_its_query() {
    let broken = [
        (
            "unscoped read",
            "SELECT id FROM ledger.account WHERE name = $1 LIMIT 10",
        ),
        (
            "tenant from a constant",
            "SELECT id FROM ledger.account \
             WHERE tenant = '00000000-0000-0000-0000-000000000000' AND name = $1",
        ),
        (
            "tenant under OR",
            "SELECT id FROM ledger.account WHERE (tenant = $1 OR name = $2) AND id = $3",
        ),
        (
            "insert without tenant",
            "INSERT INTO ledger.entry (id, account, amount, recorded_at, expires_at) \
             VALUES ($1, $2, $3, $4, $5)",
        ),
        (
            "outer join condition",
            "SELECT a.id FROM ledger.account AS a LEFT JOIN ledger.entry AS e \
             ON e.account = a.id AND e.tenant = $1 AND a.tenant = $1 WHERE a.id = $2",
        ),
        (
            "unbounded select",
            "SELECT id FROM ledger.entry WHERE tenant = $1",
        ),
        (
            "grouped select",
            "SELECT account, count(*) FROM ledger.entry WHERE tenant = $1 GROUP BY account",
        ),
        (
            "unbatched delete",
            "DELETE FROM ledger.entry WHERE tenant = $1 AND expires_at <= $2",
        ),
        (
            "update without where",
            "UPDATE ledger.account SET name = name",
        ),
        (
            "sequential scan",
            "-- veoveo: all tenants (the fixture counts every tenant's notes)\n\
             SELECT count(*) FROM ledger.entry WHERE note = $1",
        ),
        (
            "lost keyset seek",
            "SELECT id FROM ledger.account WHERE tenant = $1 \
             AND ($2::timestamptz IS NULL OR (created_at, id) > ($2, $3::uuid)) \
             ORDER BY created_at, id LIMIT $4",
        ),
        (
            "export after a joined position",
            "-- veoveo: all tenants (the exporter reads every tenant's accounts)\n\
             SELECT a.id FROM ledger.export_position AS p \
             JOIN ledger.account AS a ON (a.created_at, a.id) > (p.created_at, p.id) \
             ORDER BY a.created_at, a.id LIMIT $1",
        ),
        (
            "marker without reason",
            "-- veoveo: unbounded\nSELECT id FROM ledger.account WHERE tenant = $1 AND id = $2",
        ),
        (
            "marker with an empty reason",
            "-- veoveo: all tenants ()\nSELECT id FROM ledger.account WHERE tenant = $1 AND id = $2",
        ),
        (
            "unknown marker",
            "-- veoveo: everything (because)\n\
             SELECT id FROM ledger.account WHERE tenant = $1 AND id = $2",
        ),
        (
            "unneeded marker",
            "-- veoveo: scan (the account is read by its key)\n\
             SELECT id FROM ledger.account WHERE tenant = $1 AND id = $2",
        ),
        ("missing relation", "SELECT 1 FROM ledger.missing"),
        ("unparseable", "SELEC 1"),
    ];
    let mut check = ledger(migrator("ledger").await);
    for (label, sql) in broken {
        check = check.query(label, sql);
    }
    let report = check.run().await;
    let tenant = "has no equality on `tenant`";
    assert_reports(&report, "unscoped read", Rule::TenantPredicate, tenant);
    assert_reports(
        &report,
        "tenant from a constant",
        Rule::TenantPredicate,
        tenant,
    );
    assert_reports(&report, "tenant under OR", Rule::TenantPredicate, tenant);
    assert_reports(
        &report,
        "insert without tenant",
        Rule::TenantPredicate,
        "does not supply `tenant`",
    );
    assert_reports(
        &report,
        "outer join condition",
        Rule::TenantPredicate,
        "`ledger.account AS a` has no equality",
    );
    assert!(
        !of(&report, "outer join condition")
            .iter()
            .any(|(rule, message)| *rule == Rule::TenantPredicate
                && message.contains("ledger.entry AS e")),
        "{report}"
    );
    assert_reports(
        &report,
        "unbounded select",
        Rule::RowBound,
        "any number of rows",
    );
    assert_reports(
        &report,
        "grouped select",
        Rule::RowBound,
        "any number of rows",
    );
    assert_reports(
        &report,
        "unbatched delete",
        Rule::RowBound,
        "neither pins a primary or unique key",
    );
    assert_reports(
        &report,
        "update without where",
        Rule::RowBound,
        "has no WHERE clause",
    );
    assert_reports(&report, "update without where", Rule::IndexUse, "Seq Scan");
    assert_reports(
        &report,
        "sequential scan",
        Rule::IndexUse,
        "Seq Scan on `ledger.entry`",
    );
    assert_reports(
        &report,
        "lost keyset seek",
        Rule::IndexUse,
        "filters `created_at`, `id` instead of seeking",
    );
    assert_reports(
        &report,
        "export after a joined position",
        Rule::IndexUse,
        "on `ledger.account AS a`",
    );
    assert_reports(
        &report,
        "marker without reason",
        Rule::Marker,
        "needs a reason",
    );
    assert_reports(
        &report,
        "marker with an empty reason",
        Rule::Marker,
        "non-empty reason",
    );
    assert_reports(
        &report,
        "unknown marker",
        Rule::Marker,
        "names no marker kind",
    );
    assert_reports(&report, "unneeded marker", Rule::Marker, "is not needed");
    assert_reports(
        &report,
        "missing relation",
        Rule::Statement,
        "does not exist",
    );
    assert_reports(&report, "unparseable", Rule::Statement, "parser rejects");
    assert!(
        report
            .exemptions
            .iter()
            .any(|exemption| exemption.subject == "sequential scan"),
        "{report}"
    );
    let text = report.to_string();
    assert!(
        text.contains("Tenant predicates (exempt a query with"),
        "{text}"
    );
}

#[tokio::test]
async fn schema_rules_name_each_table_that_breaks_them() {
    let report = Conformance::new()
        .server(&slug("defects"), migrator("defects").await)
        .run()
        .await;
    assert_reports(&report, "defects.loose", Rule::Schema, "has no primary key");
    assert_reports(
        &report,
        "defects.loose",
        Rule::Schema,
        "has no comment stating",
    );
    assert_reports(
        &report,
        "defects.child",
        Rule::Schema,
        "has no comment stating",
    );
    assert_reports(
        &report,
        "defects.child",
        Rule::Schema,
        "foreign key `child_parent_fkey` (parent) has no index",
    );
    assert_eq!(of(&report, "defects.parent"), vec![], "{report}");
    assert_eq!(of(&report, "defects.optional_child"), vec![], "{report}");
    assert_eq!(report.violations.len(), 4, "{report}");
}

#[tokio::test]
async fn pending_work_is_listed_until_it_is_done() {
    let report = Conformance::new()
        .server(&slug("defects"), migrator("defects").await)
        .pending(
            Rule::Schema,
            "defects.loose",
            "the fixture keeps loose notes",
        )
        .pending(
            Rule::Schema,
            "defects.parent",
            "nothing is wrong with parent",
        )
        .run()
        .await;
    assert_eq!(of(&report, "defects.loose"), vec![], "{report}");
    assert!(
        report.exemptions.iter().any(|exemption| exemption.pending
            && exemption.subject == "defects.loose"
            && exemption.reason == "the fixture keeps loose notes"),
        "{report}"
    );
    assert_reports(
        &report,
        "defects.parent",
        Rule::Configuration,
        "names no Schema violation",
    );
    assert!(
        report.to_string().contains("pending: defects.loose"),
        "{report}"
    );
}

#[tokio::test]
async fn configuration_names_tables_and_columns_the_database_has() {
    let report = ledger(migrator("ledger").await)
        .schema("absent")
        .tenant_columns("ledger.account", ["owner"])
        .tenant_columns("ledger.nothing", ["tenant"])
        .all_tenants("ledger.setting", "the fixture reads no setting")
        .run()
        .await;
    assert_reports(
        &report,
        "absent",
        Rule::Configuration,
        "no table in this schema",
    );
    assert_reports(&report, "ledger.account", Rule::Configuration, "`owner`");
    assert_reports(
        &report,
        "ledger.nothing",
        Rule::Configuration,
        "does not have",
    );
    assert_reports(
        &report,
        "ledger.setting",
        Rule::Configuration,
        "no checked query reads",
    );
}

/// A scratch workspace with a `.sqlx` directory, removed on drop.
struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("vv-conformance-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join(".sqlx")).unwrap();
        Self(root)
    }

    /// Write `sql` as `file` (when given) and record it as compiled offline metadata.
    fn query(&self, file: Option<&str>, sql: &str) {
        if let Some(file) = file {
            let path = self.0.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, sql).unwrap();
        }
        let hash = hex::encode(Sha256::digest(sql.as_bytes()));
        let metadata = serde_json::json!({
            "db_name": "PostgreSQL",
            "query": sql,
            "describe": { "columns": [], "parameters": { "Left": [] }, "nullable": [] },
            "hash": hash,
        });
        std::fs::write(
            self.0.join(format!(".sqlx/query-{hash}.json")),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn offline_queries_are_reported_by_their_files_and_filtered_by_source() {
    let workspace = Workspace::new();
    workspace.query(
        Some("server/queries/unscoped.sql"),
        "SELECT id FROM ledger.account WHERE name = $1 LIMIT 1\n",
    );
    workspace.query(
        Some("tests/queries/count.sql"),
        "SELECT count(*) FROM ledger.account\n",
    );
    workspace.query(None, "SELECT id FROM ledger.entry WHERE note = $1 LIMIT 1");

    let everything = ledger(migrator("ledger").await)
        .queries_from(workspace.0.join(".sqlx"))
        .run()
        .await;
    assert_eq!(everything.queries, 3, "{everything}");
    assert_reports(
        &everything,
        "server/queries/unscoped.sql",
        Rule::TenantPredicate,
        "has no equality",
    );
    assert_reports(
        &everything,
        "tests/queries/count.sql",
        Rule::TenantPredicate,
        "has no equality",
    );
    assert!(
        everything.violations.iter().any(|violation| violation
            .subject
            .starts_with("SELECT id FROM ledger.entry")
            && violation.subject.contains("(query-")),
        "{everything}"
    );

    let server = ledger(migrator("ledger").await)
        .queries_from(workspace.0.join(".sqlx"))
        .sources(workspace.0.join("server"))
        .run()
        .await;
    assert_eq!(server.queries, 1, "{server}");
    assert!(
        server
            .violations
            .iter()
            .all(|violation| violation.subject == "server/queries/unscoped.sql"),
        "{server}"
    );
}

/// An owner's migrator, which keeps its history in its own schema and creates it, as an
/// owner's `sqlx.toml` declares.
async fn owner_migrator(schema: &'static str) -> Migrator {
    let mut migrator = migrator(schema).await;
    migrator.dangerous_set_table_name(format!("{schema}._sqlx_migrations"));
    migrator.create_schema(schema);
    migrator
}

#[tokio::test]
async fn an_owner_schema_is_migrated_after_the_kernel_and_checked_as_the_installation_owner() {
    let report = Conformance::new()
        .owner("inventory", owner_migrator("inventory").await)
        .query(
            "item by name",
            "SELECT id FROM inventory.item WHERE tenant = $1 AND name = $2",
        )
        .query(
            "applied migration",
            "SELECT success FROM inventory._sqlx_migrations WHERE version = $1",
        )
        .query(
            "unscoped item read",
            "SELECT id FROM inventory.item WHERE name = $1 LIMIT 10",
        )
        .run()
        .await;
    assert_eq!(of(&report, "item by name"), vec![], "{report}");
    // The history table exists in the owner's schema, so the statement plans.
    assert_eq!(of(&report, "applied migration"), vec![], "{report}");
    assert_eq!(of(&report, "inventory"), vec![], "{report}");
    assert_reports(
        &report,
        "unscoped item read",
        Rule::TenantPredicate,
        "inventory.item",
    );
}

#[tokio::test]
async fn an_owner_schema_is_held_to_the_schema_rules() {
    let report = Conformance::new()
        .owner("stockroom", owner_migrator("stockroom").await)
        .run()
        .await;
    assert_reports(&report, "stockroom.bin", Rule::Schema, "retention");
    assert!(
        !report
            .violations
            .iter()
            .any(|violation| violation.subject.contains("_sqlx_migrations")),
        "the history table is exempt from the schema rules\n{report}"
    );
}

#[tokio::test]
#[should_panic(expected = "an owner's migrations keep their history in its own schema")]
async fn an_owner_history_outside_its_schema_is_refused() {
    let _ = Conformance::new().owner("inventory", migrator("inventory").await);
}

#[tokio::test]
#[should_panic(expected = "a conformance check covers one server or one owner schema, not both")]
async fn an_owner_and_a_server_in_one_check_are_refused() {
    let _ = Conformance::new()
        .server(&slug("ledger"), migrator("ledger").await)
        .owner("inventory", owner_migrator("inventory").await);
}

#[tokio::test]
#[should_panic(expected = "is a kernel schema")]
async fn a_kernel_schema_is_not_an_owner_schema() {
    let mut kernel = migrator("inventory").await;
    kernel.dangerous_set_table_name("audit._sqlx_migrations");
    let _ = Conformance::new().owner("audit", kernel);
}
