//! The migrated database's relations, keys, indexes and functions, read once from the
//! PostgreSQL catalog.
use std::collections::{BTreeMap, HashMap};

use pg_query::{NodeEnum, protobuf as pb};
use sqlx::PgPool;

use super::expr;

pub(super) type RelationId = usize;

/// The tenant columns PostgreSQL tables use unless the configuration names others.
pub(super) const DEFAULT_TENANT_COLUMNS: [&str; 2] = ["tenant", "tenant_id"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RelationKind {
    Table,
    View,
}

#[derive(Debug)]
pub(super) struct Relation {
    pub schema: String,
    pub name: String,
    pub kind: RelationKind,
    pub partition: bool,
    pub comment: Option<String>,
    pub columns: Vec<Column>,
    pub primary_key: Option<Vec<String>>,
    /// Column sets that identify at most one row. `ctid` is always one.
    pub unique_keys: Vec<UniqueKey>,
    pub foreign_keys: Vec<ForeignKey>,
    pub indexes: Vec<Index>,
    /// A table whose primary key is one boolean column checked to be true holds at most
    /// one row.
    pub singleton: bool,
    /// The columns whose equality scopes a query to one tenant; none for a table that
    /// queries may read across tenants.
    pub tenant_columns: Vec<String>,
}

impl Relation {
    pub fn qualified(&self) -> String {
        format!("{}.{}", self.schema, self.name)
    }

    pub fn has_column(&self, name: &str) -> bool {
        name == "ctid" || self.columns.iter().any(|column| column.name == name)
    }

    /// The primary key when it has exactly one column.
    pub fn single_key(&self) -> Option<&str> {
        match self.primary_key.as_deref() {
            Some([column]) => Some(column),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub(super) struct Column {
    pub name: String,
    pub type_name: String,
    pub generated: bool,
}

/// A unique key and the columns its partial index requires to be non-null.
#[derive(Debug)]
pub(super) struct UniqueKey {
    pub columns: Vec<String>,
    pub not_null: Vec<String>,
}

#[derive(Debug)]
pub(super) struct ForeignKey {
    pub name: String,
    pub columns: Vec<String>,
}

#[derive(Debug)]
pub(super) struct Index {
    pub name: String,
    /// Key columns in order; `None` for an expression.
    pub columns: Vec<Option<String>>,
    /// The columns a partial index requires to be non-null, empty for a full index, or
    /// `None` when its predicate holds anything else.
    pub not_null: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct FunctionKind {
    pub returns_set: bool,
    pub aggregate: bool,
}

pub(super) struct Catalog {
    pub relations: Vec<Relation>,
    by_name: HashMap<(String, String), RelationId>,
    functions: HashMap<(String, String), FunctionKind>,
    search_path: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct RelationRow {
    oid: i64,
    schema: String,
    name: String,
    kind: String,
    partition: bool,
    comment: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ColumnRow {
    relation: i64,
    name: String,
    type_name: String,
    generated: bool,
}

#[derive(sqlx::FromRow)]
struct IndexRow {
    relation: i64,
    name: String,
    is_unique: bool,
    columns: Vec<Option<String>>,
    predicate: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ConstraintRow {
    relation: i64,
    name: String,
    kind: String,
    columns: Vec<String>,
    definition: String,
}

#[derive(sqlx::FromRow)]
struct FunctionRow {
    schema: String,
    name: String,
    returns_set: bool,
    aggregate: bool,
}

impl Catalog {
    /// Read the catalog through `catalog` and the search path of `session`, the
    /// connection that will plan the queries.
    pub async fn load(catalog: &PgPool, session: &PgPool) -> Result<Self, sqlx::Error> {
        let relation_rows: Vec<RelationRow> = sqlx::query_as(include_str!("catalog/relations.sql"))
            .fetch_all(catalog)
            .await?;
        let column_rows: Vec<ColumnRow> = sqlx::query_as(include_str!("catalog/columns.sql"))
            .fetch_all(catalog)
            .await?;
        let index_rows: Vec<IndexRow> = sqlx::query_as(include_str!("catalog/indexes.sql"))
            .fetch_all(catalog)
            .await?;
        let constraint_rows: Vec<ConstraintRow> =
            sqlx::query_as(include_str!("catalog/constraints.sql"))
                .fetch_all(catalog)
                .await?;
        let function_rows: Vec<FunctionRow> = sqlx::query_as(include_str!("catalog/functions.sql"))
            .fetch_all(catalog)
            .await?;
        let search_path: Vec<String> = sqlx::query_scalar(include_str!("catalog/search_path.sql"))
            .fetch_one(session)
            .await?;

        let mut relations = Vec::new();
        let mut by_oid = HashMap::new();
        for row in relation_rows {
            by_oid.insert(row.oid, relations.len());
            relations.push(Relation {
                schema: row.schema,
                name: row.name,
                kind: if matches!(row.kind.as_str(), "v" | "m") {
                    RelationKind::View
                } else {
                    RelationKind::Table
                },
                partition: row.partition,
                comment: row.comment,
                columns: Vec::new(),
                primary_key: None,
                unique_keys: vec![UniqueKey {
                    columns: vec!["ctid".to_owned()],
                    not_null: Vec::new(),
                }],
                foreign_keys: Vec::new(),
                indexes: Vec::new(),
                singleton: false,
                tenant_columns: Vec::new(),
            });
        }
        for row in column_rows {
            if let Some(&id) = by_oid.get(&row.relation) {
                relations[id].columns.push(Column {
                    name: row.name,
                    type_name: row.type_name,
                    generated: row.generated,
                });
            }
        }
        for row in index_rows {
            let Some(&id) = by_oid.get(&row.relation) else {
                continue;
            };
            let not_null = match row.predicate.as_deref() {
                None => Some(Vec::new()),
                Some(predicate) => not_null_predicate(predicate),
            };
            let relation = &mut relations[id];
            if row.is_unique
                && let (Some(columns), Some(not_null)) = (
                    row.columns.iter().cloned().collect::<Option<Vec<_>>>(),
                    not_null.clone(),
                )
            {
                relation.unique_keys.push(UniqueKey { columns, not_null });
            }
            relation.indexes.push(Index {
                name: row.name,
                columns: row.columns,
                not_null,
            });
        }
        let mut checks: BTreeMap<RelationId, Vec<String>> = BTreeMap::new();
        for row in constraint_rows {
            let Some(&id) = by_oid.get(&row.relation) else {
                continue;
            };
            match row.kind.as_str() {
                "p" => relations[id].primary_key = Some(row.columns),
                "f" => relations[id].foreign_keys.push(ForeignKey {
                    name: row.name,
                    columns: row.columns,
                }),
                _ => checks.entry(id).or_default().push(row.definition),
            }
        }
        for (id, relation) in relations.iter_mut().enumerate() {
            if let Some(column) = relation.single_key().map(str::to_owned) {
                let boolean = relation
                    .columns
                    .iter()
                    .any(|c| c.name == column && c.type_name == "boolean");
                let forced = checks
                    .get(&id)
                    .is_some_and(|checks| checks.contains(&format!("CHECK ({column})")));
                relation.singleton = boolean && forced;
            }
        }
        let by_name = relations
            .iter()
            .enumerate()
            .map(|(id, relation)| ((relation.schema.clone(), relation.name.clone()), id))
            .collect();
        let functions = function_rows
            .into_iter()
            .map(|row| {
                (
                    (row.schema, row.name),
                    FunctionKind {
                        returns_set: row.returns_set,
                        aggregate: row.aggregate,
                    },
                )
            })
            .collect();
        Ok(Self {
            relations,
            by_name,
            functions,
            search_path,
        })
    }

    /// Give every singleton table the statistics of its one row. The other tables stay
    /// unanalyzed, which the planner costs as ten pages of rows. The server's login
    /// writes the statistics in its own schema, which the installation owner cannot use.
    pub async fn size_singletons(
        &self,
        owner: &PgPool,
        server: Option<(&str, &PgPool)>,
    ) -> Result<(), sqlx::Error> {
        for relation in self.relations.iter().filter(|relation| relation.singleton) {
            let pool = match server {
                Some((schema, pool)) if relation.schema == schema => pool,
                _ => owner,
            };
            sqlx::query(include_str!("catalog/singleton_statistics.sql"))
                .bind(&relation.schema)
                .bind(&relation.name)
                .execute(pool)
                .await?;
        }
        Ok(())
    }

    pub fn relation(&self, id: RelationId) -> &Relation {
        &self.relations[id]
    }

    /// The relation a name denotes: a qualified name directly, an unqualified one
    /// through the planning session's search path.
    pub fn resolve(&self, schema: Option<&str>, name: &str) -> Option<RelationId> {
        match schema {
            Some(schema) => self
                .by_name
                .get(&(schema.to_owned(), name.to_owned()))
                .copied(),
            None => self
                .search_path
                .iter()
                .find_map(|schema| self.by_name.get(&(schema.clone(), name.to_owned())))
                .copied(),
        }
    }

    /// The relation named `schema.name`.
    pub fn named(&self, qualified: &str) -> Option<RelationId> {
        let (schema, name) = qualified.split_once('.')?;
        self.resolve(Some(schema), name)
    }

    pub fn function(&self, schema: Option<&str>, name: &str) -> Option<FunctionKind> {
        match schema {
            Some(schema) => self
                .functions
                .get(&(schema.to_owned(), name.to_owned()))
                .copied(),
            None => self
                .search_path
                .iter()
                .find_map(|schema| self.functions.get(&(schema.clone(), name.to_owned())))
                .copied(),
        }
    }

    /// The index named `name` in `schema`, with the relation it belongs to.
    pub fn index(&self, schema: &str, name: &str) -> Option<(&Relation, &Index)> {
        self.relations
            .iter()
            .filter(|relation| relation.schema == schema)
            .find_map(|relation| {
                relation
                    .indexes
                    .iter()
                    .find(|index| index.name == name)
                    .map(|index| (relation, index))
            })
    }

    /// Give every relation its tenant columns: the configured ones, or the default names
    /// it has. A relation in `all_tenants` has none.
    pub fn assign_tenants(
        &mut self,
        configured: &BTreeMap<RelationId, Vec<String>>,
        all_tenants: &BTreeMap<RelationId, String>,
    ) {
        for (id, relation) in self.relations.iter_mut().enumerate() {
            relation.tenant_columns = match configured.get(&id) {
                _ if all_tenants.contains_key(&id) => Vec::new(),
                Some(columns) => columns.clone(),
                None => DEFAULT_TENANT_COLUMNS
                    .iter()
                    .filter(|name| relation.has_column(name))
                    .map(|name| (*name).to_owned())
                    .collect(),
            };
        }
    }
}

/// The columns a partial index predicate requires to be non-null, when the predicate is
/// nothing but `column IS NOT NULL` tests joined by `AND`.
fn not_null_predicate(predicate: &str) -> Option<Vec<String>> {
    let parsed = pg_query::parse(&format!("SELECT 1 WHERE {predicate}")).ok()?;
    let statement = parsed.protobuf.stmts.first()?.stmt.as_deref()?;
    let Some(NodeEnum::SelectStmt(select)) = &statement.node else {
        return None;
    };
    expr::conjuncts(select.where_clause.as_deref()?)
        .into_iter()
        .map(|conjunct| match &conjunct.node {
            Some(NodeEnum::NullTest(test))
                if test.nulltesttype == pb::NullTestType::IsNotNull as i32 =>
            {
                match test.arg.as_deref().and_then(|arg| arg.node.as_ref()) {
                    Some(NodeEnum::ColumnRef(column)) => {
                        match expr::names(&column.fields).as_slice() {
                            [name] => Some((*name).to_owned()),
                            _ => None,
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}
