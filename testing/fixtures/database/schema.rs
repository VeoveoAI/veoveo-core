//! Schema rules from the catalog: primary keys, indexed foreign keys and retention
//! comments.
use std::collections::BTreeSet;

use super::catalog::{Catalog, RelationKind};

/// The prefix of a table comment that states the table's retention.
pub(super) const RETENTION: &str = "Retention:";

/// `(table, message)` for every rule a table of `schemas` breaks.
pub(super) fn check(catalog: &Catalog, schemas: &BTreeSet<String>) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for relation in &catalog.relations {
        if relation.kind != RelationKind::Table
            || relation.partition
            || !schemas.contains(&relation.schema)
            || relation.name == "_sqlx_migrations"
        {
            continue;
        }
        let table = relation.qualified();
        if relation.primary_key.is_none() {
            found.push((table.clone(), "has no primary key".to_owned()));
        }
        if !relation.singleton {
            for key in &relation.foreign_keys {
                let wanted: BTreeSet<&str> = key.columns.iter().map(String::as_str).collect();
                let indexed = relation.indexes.iter().any(|index| {
                    let leading: Option<BTreeSet<&str>> = index
                        .columns
                        .iter()
                        .take(wanted.len())
                        .map(|column| column.as_deref())
                        .collect();
                    let usable = index.not_null.as_ref().is_some_and(|required| {
                        required
                            .iter()
                            .all(|column| wanted.contains(column.as_str()))
                    });
                    usable && leading.as_ref() == Some(&wanted)
                });
                if !indexed {
                    found.push((
                        table.clone(),
                        format!(
                            "foreign key `{}` ({}) has no index whose leading columns are its \
                             columns",
                            key.name,
                            key.columns.join(", ")
                        ),
                    ));
                }
            }
        }
        let states_retention = relation.comment.as_deref().is_some_and(|comment| {
            comment
                .strip_prefix(RETENTION)
                .is_some_and(|rest| !rest.trim().is_empty())
        });
        if !states_retention {
            found.push((
                table.clone(),
                format!(
                    "has no comment stating its retention, as `COMMENT ON TABLE {table} IS \
                     '{RETENTION} …'`"
                ),
            ));
        }
    }
    found
}
