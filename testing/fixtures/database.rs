//! Database conformance checks for a crate's compile-checked queries and its schema.
//!
//! [`Conformance`] migrates a fixture database, reads every query of a sqlx offline
//! directory and checks four rules: tenant predicates, row bounds, index use under
//! generic plans and the schema rules of `docs/DATABASE.md`. One call reports every
//! violation and every exemption. Requires the PostgreSQL 18 server programs on `PATH`.
//!
//! ```ignore
//! veoveo_test_fixtures::database::Conformance::new()
//!     .server(&ServerSlug::parse("time").unwrap(), persistence::migrator())
//!     .queries_from(concat!(env!("CARGO_MANIFEST_DIR"), "/.sqlx"))
//!     .check()
//!     .await;
//! ```
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};

use sqlx::migrate::Migrator;
use veoveo_platform_store::ServerSchema;
use veoveo_types::ServerSlug;

use super::store::TestDb;

#[path = "database/catalog.rs"]
mod catalog;
#[path = "database/expr.rs"]
mod expr;
#[path = "database/markers.rs"]
mod markers;
#[path = "database/plan.rs"]
mod plan;
#[path = "database/queries.rs"]
mod queries;
#[path = "database/report.rs"]
mod report;
#[path = "database/schema.rs"]
mod schema;
#[path = "database/scope.rs"]
mod scope;
#[path = "database/statement.rs"]
mod statement;

pub use report::{Exemption, Report, Rule, Violation};

/// The longest a whole check may take, including the fixture database.
const LIMIT: Duration = Duration::from_secs(300);

/// A conformance check of one crate's queries and schemas.
#[must_use = "a conformance check runs only when awaited through `check` or `run`"]
#[derive(Default)]
pub struct Conformance {
    schemas: BTreeSet<String>,
    server: Option<(ServerSlug, Migrator)>,
    directories: Vec<PathBuf>,
    sources: Vec<PathBuf>,
    inline: Vec<(String, String)>,
    tenant_columns: BTreeMap<String, Vec<String>>,
    all_tenants: BTreeMap<String, String>,
    pending: Vec<(Rule, String, String)>,
}

impl Conformance {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check the tables of `schema`: index use by the queries and the schema rules.
    pub fn schema(mut self, schema: impl Into<String>) -> Self {
        self.schemas.insert(schema.into());
        self
    }

    /// Apply a server's migrations in its own schema, as its login does at startup,
    /// check that schema, and plan every query as one of the server's logins. Without a
    /// server, queries are planned as the installation owner.
    pub fn server(mut self, server: &ServerSlug, migrator: Migrator) -> Self {
        assert!(
            self.server.is_none(),
            "a conformance check covers one server"
        );
        let schema = ServerSchema::new(server).expect("a server slug names a server schema");
        self.schemas.insert(schema.schema().to_owned());
        self.server = Some((server.clone(), migrator));
        self
    }

    /// Check every query of a sqlx offline directory, usually the workspace's `.sqlx`.
    /// Each query is reported by the `.sql` file under the directory's parent whose
    /// contents it is.
    pub fn queries_from(mut self, directory: impl AsRef<Path>) -> Self {
        self.directories.push(directory.as_ref().to_owned());
        self
    }

    /// Keep only the queries whose `.sql` file lies under `directory`, such as this
    /// crate's `src`. Repeating it adds directories.
    pub fn sources(mut self, directory: impl AsRef<Path>) -> Self {
        self.sources.push(directory.as_ref().to_owned());
        self
    }

    /// Check one more statement, reported under `label`.
    pub fn query(mut self, label: impl Into<String>, sql: impl Into<String>) -> Self {
        self.inline.push((label.into(), sql.into()));
        self
    }

    /// The columns that scope `table` (`schema.name`) to one tenant, in place of the
    /// default `tenant` and `tenant_id`, such as `partition` for `audit.record`.
    pub fn tenant_columns<I, S>(mut self, table: &str, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tenant_columns.insert(
            table.to_owned(),
            columns.into_iter().map(Into::into).collect(),
        );
        self
    }

    /// Queries may read `table` across tenants, for `reason`. The report lists the
    /// exemption.
    pub fn all_tenants(mut self, table: &str, reason: &str) -> Self {
        self.all_tenants.insert(table.to_owned(), reason.to_owned());
        self
    }

    /// Accept the violations of `rule` under `subject`, a query file or a table, as known
    /// work that `reason` names, such as a schema defect the owning change has not fixed
    /// yet. The report lists them as pending, and an entry that matches no violation is
    /// itself one, so it leaves with the defect.
    pub fn pending(mut self, rule: Rule, subject: &str, reason: &str) -> Self {
        self.pending
            .push((rule, subject.to_owned(), reason.to_owned()));
        self
    }

    /// Run the check and panic with the report when any rule is broken. The report of
    /// a passing check, with its exemptions, goes to standard error.
    pub async fn check(self) {
        let report = self.run().await;
        if report.is_conformant() {
            eprintln!("{report}");
        } else {
            panic!("{report}");
        }
    }

    /// Run the check and return the report.
    pub async fn run(self) -> Report {
        tokio::time::timeout(LIMIT, self.evaluate())
            .await
            .unwrap_or_else(|_| panic!("Database conformance exceeded {LIMIT:?}"))
    }

    async fn evaluate(self) -> Report {
        let Self {
            schemas,
            server,
            directories,
            sources,
            inline,
            tenant_columns,
            all_tenants,
            pending,
        } = self;
        let queries = queries::load(&directories, &sources, &inline)
            .unwrap_or_else(|error| panic!("Database conformance queries: {error}"));
        let db = TestDb::new().await;
        let server_schema = server.as_ref().map(|(server, _)| {
            ServerSchema::new(server)
                .expect("a server slug names a server schema")
                .schema()
                .to_owned()
        });
        let planner = match server {
            Some((server, migrator)) => {
                db.migrate_server(&server, migrator).await;
                db.server_store(&server).await
            }
            None => db.connect().await,
        };
        let mut catalog = catalog::Catalog::load(db.a.pool(), planner.pool())
            .await
            .unwrap_or_else(|error| panic!("Database conformance catalog: {error}"));
        let mut violations = Vec::new();
        let mut exemptions = Vec::new();
        let mut violate = |rule, subject: &str, message: String| {
            violations.push(Violation {
                rule,
                subject: subject.to_owned(),
                message,
            });
        };

        for schema in &schemas {
            if !catalog
                .relations
                .iter()
                .any(|relation| &relation.schema == schema)
            {
                violate(
                    Rule::Configuration,
                    schema,
                    "the migrated database has no table in this schema".to_owned(),
                );
            }
        }
        let mut configured = BTreeMap::new();
        for (table, columns) in &tenant_columns {
            let Some(id) = catalog.named(table) else {
                violate(
                    Rule::Configuration,
                    table,
                    "`tenant_columns` names a table the migrated database does not have".to_owned(),
                );
                continue;
            };
            for column in columns {
                if !catalog.relation(id).has_column(column) {
                    violate(
                        Rule::Configuration,
                        table,
                        format!("`tenant_columns` names `{column}`, which the table does not have"),
                    );
                }
            }
            configured.insert(id, columns.clone());
        }
        let mut unscoped = BTreeMap::new();
        for (table, reason) in &all_tenants {
            match catalog.named(table) {
                Some(id) => {
                    unscoped.insert(id, reason.clone());
                }
                None => violate(
                    Rule::Configuration,
                    table,
                    "`all_tenants` names a table the migrated database does not have".to_owned(),
                ),
            }
        }
        catalog.assign_tenants(&configured, &unscoped);

        // The fixture's tables are empty and never analyzed, so the planner costs each
        // as ten pages of rows rather than as an empty table; a singleton gets its one row.
        let server_schema = server_schema
            .as_deref()
            .map(|schema| (schema, planner.pool()));
        catalog
            .size_singletons(db.a.pool(), server_schema)
            .await
            .unwrap_or_else(|error| panic!("Database conformance statistics: {error}"));
        let mut connection = planner
            .pool()
            .acquire()
            .await
            .unwrap_or_else(|error| panic!("Database conformance planner session: {error}"));
        sqlx::raw_sql("SET enable_seqscan = off")
            .execute(&mut *connection)
            .await
            .unwrap_or_else(|error| panic!("Database conformance planner session: {error}"));
        let mut referenced = BTreeSet::new();
        for query in &queries {
            let analysis = match statement::analyze(&catalog, &query.text) {
                Ok(analysis) => analysis,
                Err(error) => {
                    violate(
                        Rule::Statement,
                        &query.subject,
                        format!("PostgreSQL's parser rejects the query: {error}"),
                    );
                    continue;
                }
            };
            referenced.extend(analysis.referenced.iter().copied());
            let mut findings = analysis.findings;
            for text in &analysis.plannable {
                match plan::explain(&mut connection, text).await {
                    Ok(plan) => findings.extend(
                        plan::scans(&plan, &catalog, &schemas)
                            .into_iter()
                            .map(|scan| (Rule::IndexUse, scan)),
                    ),
                    Err(error) => violate(
                        Rule::Statement,
                        &query.subject,
                        format!("EXPLAIN fails: {error}"),
                    ),
                }
            }
            let (markers, problems) = markers::parse(&query.text);
            for problem in problems {
                violate(Rule::Marker, &query.subject, problem);
            }
            for marker in markers {
                let rule = marker.kind.rule();
                if findings.iter().any(|(found, _)| *found == rule) {
                    findings.retain(|(found, _)| *found != rule);
                    exemptions.push(Exemption {
                        rule,
                        subject: query.subject.clone(),
                        reason: marker.reason,
                        pending: false,
                    });
                } else {
                    violate(
                        Rule::Marker,
                        &query.subject,
                        format!(
                            "`-- veoveo: {} (…)` is not needed: the query passes the check it \
                             exempts",
                            marker.kind.spelling()
                        ),
                    );
                }
            }
            for (rule, message) in findings {
                violate(rule, &query.subject, message);
            }
        }
        drop(connection);
        planner.close().await;

        for (id, reason) in &unscoped {
            let table = catalog.relation(*id).qualified();
            if referenced.contains(id) {
                exemptions.push(Exemption {
                    rule: Rule::TenantPredicate,
                    subject: table,
                    reason: reason.clone(),
                    pending: false,
                });
            } else {
                violate(
                    Rule::Configuration,
                    &table,
                    "`all_tenants` names a table that no checked query reads".to_owned(),
                );
            }
        }
        for (table, message) in schema::check(&catalog, &schemas) {
            violate(Rule::Schema, &table, message);
        }
        for (rule, subject, reason) in pending {
            let before = violations.len();
            violations.retain(|violation| violation.rule != rule || violation.subject != subject);
            if violations.len() == before {
                violations.push(Violation {
                    rule: Rule::Configuration,
                    subject,
                    message: format!("`pending` names no {rule:?} violation; remove it"),
                });
            } else {
                exemptions.push(Exemption {
                    rule,
                    subject,
                    reason,
                    pending: true,
                });
            }
        }

        violations.sort();
        violations.dedup();
        exemptions.sort();
        Report {
            queries: queries.len(),
            tables: catalog
                .relations
                .iter()
                .filter(|relation| schemas.contains(&relation.schema))
                .count(),
            schemas: schemas.into_iter().collect(),
            violations,
            exemptions,
        }
    }
}
