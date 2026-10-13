//! Index use: the generic plan of each statement with sequential scans disabled.
use std::collections::BTreeSet;

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
/// scans, index scans without an index condition outside an ordered top-N read, and
/// index scans under a LIMIT that filter an index column instead of seeking it.
pub(super) fn scans(plan: &Value, catalog: &Catalog, schemas: &BTreeSet<String>) -> Vec<String> {
    let mut found = Vec::new();
    if let Some(root) = plan.get(0).and_then(|explained| explained.get("Plan")) {
        visit(root, false, catalog, schemas, &mut found);
    }
    found
}

fn visit(
    node: &Value,
    limited: bool,
    catalog: &Catalog,
    schemas: &BTreeSet<String>,
    found: &mut Vec<String>,
) {
    let text = |field: &str| node.get(field).and_then(Value::as_str);
    let kind = text("Node Type").unwrap_or_default();
    let limited = limited || kind == "Limit";
    if let (Some(schema), Some(table)) = (text("Schema"), text("Relation Name"))
        && schemas.contains(schema)
        && !catalog
            .resolve(Some(schema), table)
            .is_some_and(|id| catalog.relation(id).singleton)
    {
        let label = match text("Alias") {
            Some(alias) if alias != table => format!("`{schema}.{table} AS {alias}`"),
            _ => format!("`{schema}.{table}`"),
        };
        let index = text("Index Name").unwrap_or_default();
        let filter = text("Filter");
        match (kind, text("Index Cond")) {
            ("Seq Scan", _) => found.push(format!("Seq Scan on {label}")),
            ("Index Scan" | "Index Only Scan", None) if !limited || filter.is_some() => {
                found.push(format!(
                    "{kind} using `{index}` on {label} has no index condition"
                ));
            }
            ("Index Scan" | "Index Only Scan", Some(condition)) if limited => {
                lost_seek(
                    &[(index, condition)],
                    filter,
                    schema,
                    &label,
                    catalog,
                    found,
                );
            }
            ("Bitmap Heap Scan", _) if limited => {
                let mut indexes = Vec::new();
                bitmap_indexes(node, &mut indexes);
                lost_seek(&indexes, filter, schema, &label, catalog, found);
            }
            _ => {}
        }
    }
    for child in node
        .get("Plans")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        visit(child, limited, catalog, schemas, found);
    }
}

/// Report a scan under a LIMIT whose filter tests a key column of an index it reads by,
/// although no index condition seeks that column: under a generic plan the predicate,
/// typically an optional keyset bound, no longer narrows the index range.
fn lost_seek(
    indexes: &[(&str, &str)],
    filter: Option<&str>,
    schema: &str,
    label: &str,
    catalog: &Catalog,
    found: &mut Vec<String>,
) {
    let Some(filter) = filter else {
        return;
    };
    let filtered = qualified_columns(filter);
    let sought: BTreeSet<String> = indexes
        .iter()
        .flat_map(|(_, condition)| qualified_columns(condition))
        .collect();
    for (index, _) in indexes {
        let keys: Vec<String> = catalog
            .index(schema, index)
            .map(|(_, index)| index.columns.iter().flatten().cloned().collect())
            .unwrap_or_default();
        let lost: Vec<String> = keys
            .into_iter()
            .filter(|key| filtered.contains(key) && !sought.contains(key))
            .map(|key| format!("`{key}`"))
            .collect();
        if !lost.is_empty() {
            found.push(format!(
                "a scan of {label} through `{index}` under a LIMIT filters {} instead of \
                 seeking it in the index",
                lost.join(", ")
            ));
        }
    }
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

/// The column names in an EXPLAIN VERBOSE expression, where every column is qualified
/// by its relation: each name that follows a `.`.
fn qualified_columns(expression: &str) -> BTreeSet<String> {
    let mut columns = BTreeSet::new();
    let Ok(scanned) = pg_query::scan(expression) else {
        return columns;
    };
    let mut after_dot = false;
    for token in &scanned.tokens {
        let word = token.token == pb::Token::Ident as i32
            || token.keyword_kind != pb::KeywordKind::NoKeyword as i32;
        if after_dot
            && word
            && let Some(name) = expression.get(token.start as usize..token.end as usize)
        {
            columns.insert(name.trim_matches('"').to_owned());
        }
        after_dot = token.token == pb::Token::Ascii46 as i32;
    }
    columns
}
