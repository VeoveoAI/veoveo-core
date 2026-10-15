//! Index use: the generic plan of each statement with sequential scans disabled.
use std::collections::{BTreeMap, BTreeSet};

use pg_query::protobuf as pb;
use serde_json::Value;
use sqlx::{PgConnection, Row};

use super::catalog::Catalog;

/// The generic plan PostgreSQL would cache for `statement`, as EXPLAIN's JSON.
pub(super) async fn explain(
    connection: &mut PgConnection,
    statement: &str,
) -> Result<Value, String> {
    // The statement is one of the repository's own compile-checked queries, which
    // PostgreSQL plans without running.
    let explain = format!("EXPLAIN (GENERIC_PLAN, VERBOSE, FORMAT JSON) {statement}");
    let row = sqlx::raw_sql(sqlx::AssertSqlSafe(explain))
        .fetch_one(&mut *connection)
        .await
        .map_err(|error| error.to_string())?;
    row.try_get::<sqlx::types::JsonValue, _>(0)
        .map_err(|error| error.to_string())
}

/// The scans of tables in `schemas` that read rows without an index seek: sequential
/// scans, index scans without an index condition, and index scans under a LIMIT that
/// filter an index column instead of seeking it. A nested loop's join filter tests the
/// rows of its outer scan, so under a LIMIT it counts as that scan's filter.
///
/// An index scan without a condition still counts as a seek path in two places. Under
/// a LIMIT with no filter it is a top-N read in index order. As the input of a hash or
/// merge join it reads the whole table only because the planner preferred a join over
/// per-row lookups for the fixture's table sizes; when the join key leads an index of
/// that table, a nested loop can seek it instead.
pub(super) fn scans(plan: &Value, catalog: &Catalog, schemas: &BTreeSet<String>) -> Vec<String> {
    let mut found = Vec::new();
    if let Some(root) = plan.get(0).and_then(|explained| explained.get("Plan")) {
        let mut checks = Checks {
            catalog,
            schemas,
            found: &mut found,
        };
        checks.visit(root, Above::default());
    }
    found
}

/// The join-key columns of each relation a hash or merge join compares, by alias.
type JoinKeys = BTreeMap<String, BTreeSet<String>>;

/// What the plan nodes above a node say about it.
#[derive(Clone, Copy, Default)]
struct Above<'p> {
    /// A LIMIT stops the read early.
    limited: bool,
    /// The hash or merge join this node feeds compares these keys.
    joined: Option<&'p JoinKeys>,
    /// The nested loop whose outer input this node is filters its rows with this.
    join_filter: Option<&'p str>,
}

struct Checks<'c> {
    catalog: &'c Catalog,
    schemas: &'c BTreeSet<String>,
    found: &'c mut Vec<String>,
}

impl Checks<'_> {
    fn visit(&mut self, node: &Value, above: Above<'_>) {
        let text = |field: &str| node.get(field).and_then(Value::as_str);
        let kind = text("Node Type").unwrap_or_default();
        let limited = above.limited || kind == "Limit";
        self.scan(node, kind, Above { limited, ..above });
        let keys = match kind {
            "Hash Join" => text("Hash Cond").map(join_keys),
            "Merge Join" => text("Merge Cond").map(join_keys),
            _ => None,
        };
        for child in node
            .get("Plans")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let outer = child.get("Parent Relationship").and_then(Value::as_str) == Some("Outer");
            let child_above = Above {
                limited,
                // A hash join's inner input is the Hash node above the scan.
                joined: match kind {
                    "Hash Join" | "Merge Join" => keys.as_ref(),
                    "Hash" => above.joined,
                    _ => None,
                },
                join_filter: match kind {
                    "Nested Loop" if limited && outer => text("Join Filter"),
                    _ => None,
                },
            };
            self.visit(child, child_above);
        }
    }

    fn scan(&mut self, node: &Value, kind: &str, above: Above<'_>) {
        let text = |field: &str| node.get(field).and_then(Value::as_str);
        let (Some(schema), Some(table)) = (text("Schema"), text("Relation Name")) else {
            return;
        };
        let Some(id) = self.catalog.resolve(Some(schema), table) else {
            return;
        };
        if !self.schemas.contains(schema) || self.catalog.relation(id).singleton {
            return;
        }
        let alias = text("Alias").unwrap_or(table);
        let label = if alias == table {
            format!("`{schema}.{table}`")
        } else {
            format!("`{schema}.{table} AS {alias}`")
        };
        let index = text("Index Name").unwrap_or_default();
        let filtered: BTreeSet<String> = text("Filter")
            .into_iter()
            .chain(above.join_filter)
            .flat_map(qualified_references)
            .filter(|(relation, _)| relation == alias)
            .map(|(_, column)| column)
            .collect();
        let seekable_join = above
            .joined
            .and_then(|keys| keys.get(alias))
            .is_some_and(|keys| {
                self.catalog.relation(id).indexes.iter().any(|index| {
                    index.not_null.is_some()
                        && index
                            .columns
                            .first()
                            .and_then(Option::as_ref)
                            .is_some_and(|leading| keys.contains(leading))
                })
            });
        match (kind, text("Index Cond")) {
            ("Seq Scan", _) => self.found.push(format!("Seq Scan on {label}")),
            ("Index Scan" | "Index Only Scan", None)
                if !seekable_join && (!above.limited || !filtered.is_empty()) =>
            {
                self.found.push(format!(
                    "{kind} using `{index}` on {label} has no index condition"
                ));
            }
            ("Index Scan" | "Index Only Scan", Some(condition)) if above.limited => {
                self.lost_seek(&[(index, condition)], &filtered, schema, &label);
            }
            ("Bitmap Heap Scan", _) if above.limited => {
                let mut indexes = Vec::new();
                bitmap_indexes(node, &mut indexes);
                self.lost_seek(&indexes, &filtered, schema, &label);
            }
            _ => {}
        }
    }

    /// Report a scan under a LIMIT whose filter tests a key column of an index it reads
    /// by, although no index condition seeks that column: under a generic plan the
    /// predicate, typically an optional keyset bound, no longer narrows the index range.
    fn lost_seek(
        &mut self,
        indexes: &[(&str, &str)],
        filtered: &BTreeSet<String>,
        schema: &str,
        label: &str,
    ) {
        let sought: BTreeSet<String> = indexes
            .iter()
            .flat_map(|(_, condition)| qualified_references(condition))
            .map(|(_, column)| column)
            .collect();
        for (index, _) in indexes {
            let keys: Vec<String> = self
                .catalog
                .index(schema, index)
                .map(|(_, index)| index.columns.iter().flatten().cloned().collect())
                .unwrap_or_default();
            let lost: Vec<String> = keys
                .into_iter()
                .filter(|key| filtered.contains(key) && !sought.contains(key))
                .map(|key| format!("`{key}`"))
                .collect();
            if !lost.is_empty() {
                self.found.push(format!(
                    "a scan of {label} through `{index}` under a LIMIT filters {} instead of \
                     seeking it in the index",
                    lost.join(", ")
                ));
            }
        }
    }
}

/// The columns a join condition compares, grouped by the alias that qualifies them.
fn join_keys(condition: &str) -> JoinKeys {
    let mut keys = JoinKeys::new();
    for (alias, column) in qualified_references(condition) {
        keys.entry(alias).or_default().insert(column);
    }
    keys
}

/// The indexes and conditions of the Bitmap Index Scans below a Bitmap Heap Scan.
fn bitmap_indexes<'p>(node: &'p Value, indexes: &mut Vec<(&'p str, &'p str)>) {
    for child in node
        .get("Plans")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |field: &str| child.get(field).and_then(Value::as_str);
        if text("Node Type") == Some("Bitmap Index Scan")
            && let (Some(index), Some(condition)) = (text("Index Name"), text("Index Cond"))
        {
            indexes.push((index, condition));
        }
        bitmap_indexes(child, indexes);
    }
}

/// The `(relation, column)` references of an EXPLAIN VERBOSE expression, where every
/// column is qualified by its relation's alias: each pair of names joined by a `.`.
fn qualified_references(expression: &str) -> Vec<(String, String)> {
    let mut references = Vec::new();
    let Ok(scanned) = pg_query::scan(expression) else {
        return references;
    };
    let name = |token: &pb::ScanToken| {
        let word = token.token == pb::Token::Ident as i32
            || token.keyword_kind != pb::KeywordKind::NoKeyword as i32;
        word.then(|| expression.get(token.start as usize..token.end as usize))
            .flatten()
            .map(|text| text.trim_matches('"').to_owned())
    };
    for window in scanned.tokens.windows(3) {
        if window[1].token == pb::Token::Ascii46 as i32
            && let (Some(relation), Some(column)) = (name(&window[0]), name(&window[2]))
        {
            references.push((relation, column));
        }
    }
    references
}
