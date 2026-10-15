//! The relations of one query level, the equalities among their columns, and the two
//! facts the checks derive from them: which relations are scoped to a tenant and which
//! return a bounded number of rows.
use std::ops::Range;

use pg_query::protobuf as pb;

use super::catalog::{Catalog, RelationId};

/// What a value is known to be.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Provenance {
    /// A bind parameter, or the tenant of a tenant-scoped row.
    pub tenant: bool,
    /// The single-column primary key of a tenant-scoped row of this table.
    pub key_of: Option<RelationId>,
}

impl Provenance {
    pub const PARAMETER: Self = Self {
        tenant: true,
        key_of: None,
    };

    fn union(self, other: Self) -> Self {
        Self {
            tenant: self.tenant || other.tenant,
            key_of: self.key_of.or(other.key_of),
        }
    }

    /// What two branches of a set operation both know about a column.
    pub fn intersect(self, other: Self) -> Self {
        Self {
            tenant: self.tenant && other.tenant,
            key_of: self.key_of.filter(|key| other.key_of == Some(*key)),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct OutputColumn {
    pub name: String,
    pub provenance: Provenance,
}

/// A subquery, WITH query, function or VALUES list as the enclosing query sees it.
#[derive(Debug, Clone, Default)]
pub(super) struct Derived {
    pub bounded: bool,
    pub columns: Vec<OutputColumn>,
    /// Why the rows are not bounded, for the report.
    pub unbounded: Vec<String>,
}

impl Derived {
    pub fn bounded(columns: Vec<OutputColumn>) -> Self {
        Self {
            bounded: true,
            columns,
            unbounded: Vec::new(),
        }
    }

    pub fn unbounded(reason: String) -> Self {
        Self {
            bounded: false,
            columns: Vec::new(),
            unbounded: vec![reason],
        }
    }
}

#[derive(Debug, Clone)]
pub(super) enum Source {
    Table(RelationId),
    Derived(Derived),
}

/// One relation of a FROM list, a DML target or a USING list.
#[derive(Debug, Clone)]
pub(super) struct Rel {
    /// The name that qualifies its columns: the alias, or the relation's own name.
    pub name: String,
    pub source: Source,
    /// The relation as the report names it.
    pub label: String,
}

#[derive(Debug, Clone)]
pub(super) enum Operand {
    /// A column of relation `rel` of query level `frame`.
    Column {
        frame: usize,
        rel: usize,
        column: String,
    },
    Parameter,
    /// A value fixed for the whole statement: a constant, an expression over parameters
    /// and constants, or an uncorrelated scalar subquery.
    Fixed(Provenance),
    /// A bounded list of fixed values, such as `= ANY ($1)` or `IN (SELECT … LIMIT n)`,
    /// with what an uncorrelated subquery's values are known to be.
    List(Provenance),
    Other,
}

/// An equality between two operands, which constrains only the relations in `scope`:
/// every relation for WHERE and inner joins, the nullable side for outer joins.
#[derive(Debug, Clone)]
pub(super) struct Equality {
    pub left: Operand,
    pub right: Operand,
    pub scope: Range<usize>,
}

/// One query level while it is analyzed.
#[derive(Debug, Default)]
pub(super) struct Frame<'a> {
    pub rels: Vec<Rel>,
    pub equalities: Vec<Equality>,
    /// Columns the WHERE clause tests with `IS NOT NULL`.
    pub not_null: Vec<(usize, String)>,
    /// Relations scoped to one tenant.
    pub constrained: Vec<bool>,
    /// Relations that contribute a bounded number of rows.
    pub bounded: Vec<bool>,
    /// Subqueries in expressions, analyzed once the level is solved.
    pub sublinks: Vec<&'a pb::SubLink>,
    /// LATERAL subqueries, analyzed again once the level is solved.
    pub laterals: Vec<&'a pb::Node>,
}

/// Solve the top level: which of its relations are tenant-scoped and which are bounded.
/// Both grow from nothing until no equality adds a relation, so no relation justifies
/// itself.
pub(super) fn solve(frames: &mut [Frame<'_>], catalog: &Catalog) {
    let top = frames.len() - 1;
    let count = frames[top].rels.len();
    frames[top].constrained = vec![false; count];
    frames[top].bounded = frames[top]
        .rels
        .iter()
        .map(|rel| match &rel.source {
            Source::Derived(derived) => derived.bounded,
            Source::Table(id) => catalog.relation(*id).singleton,
        })
        .collect();
    loop {
        let mut changed = false;
        for rel in 0..count {
            if !frames[top].constrained[rel] && tenant_scoped(frames, catalog, top, rel) {
                frames[top].constrained[rel] = true;
                changed = true;
            }
            if !frames[top].bounded[rel] && keyed(frames, catalog, top, rel) {
                frames[top].bounded[rel] = true;
                changed = true;
            }
        }
        if !changed {
            return;
        }
    }
}

/// The equalities of `frame` that may constrain `rel`, each as (its side, other side).
fn sides<'f>(
    frames: &'f [Frame<'_>],
    frame: usize,
    rel: usize,
) -> impl Iterator<Item = (&'f str, &'f Operand)> {
    frames[frame]
        .equalities
        .iter()
        .filter(move |equality| equality.scope.contains(&rel))
        .flat_map(|equality| [(&equality.left, &equality.right), (&equality.right, &equality.left)])
        .filter_map(move |(mine, other)| match mine {
            Operand::Column {
                frame: f,
                rel: r,
                column,
            } if *f == frame && *r == rel => {
                let same = matches!(other, Operand::Column { frame: f, rel: r, .. } if *f == frame && *r == rel);
                (!same).then_some((column.as_str(), other))
            }
            _ => None,
        })
}

fn tenant_scoped(frames: &[Frame<'_>], catalog: &Catalog, top: usize, rel: usize) -> bool {
    let Source::Table(id) = frames[top].rels[rel].source else {
        return false;
    };
    let relation = catalog.relation(id);
    sides(frames, top, rel).any(|(column, other)| {
        let value = provenance(frames, catalog, other);
        (value.tenant
            && relation
                .tenant_columns
                .iter()
                .any(|tenant| tenant == column))
            || (value.key_of == Some(id) && relation.single_key() == Some(column))
    })
}

fn keyed(frames: &[Frame<'_>], catalog: &Catalog, top: usize, rel: usize) -> bool {
    let Source::Table(id) = frames[top].rels[rel].source else {
        return false;
    };
    let pinned = |column: &str| {
        sides(frames, top, rel).any(|(mine, other)| {
            mine == column
                && match other {
                    Operand::Parameter | Operand::Fixed(_) | Operand::List(_) => true,
                    Operand::Column { frame, rel: by, .. } => {
                        *frame < top || (*by != rel && frames[top].bounded[*by])
                    }
                    Operand::Other => false,
                }
        })
    };
    let tested = |column: &str| {
        frames[top]
            .not_null
            .iter()
            .any(|(tested, name)| *tested == rel && name == column)
    };
    catalog.relation(id).unique_keys.iter().any(|key| {
        key.columns.iter().all(|column| pinned(column))
            && key
                .not_null
                .iter()
                .all(|column| pinned(column) || tested(column))
    })
}

/// What a column of a relation is by itself, without the equalities around it.
fn own(
    frames: &[Frame<'_>],
    catalog: &Catalog,
    frame: usize,
    rel: usize,
    column: &str,
) -> Provenance {
    let level = &frames[frame];
    match &level.rels[rel].source {
        Source::Table(id) => {
            let relation = catalog.relation(*id);
            let scoped = level.constrained.get(rel).copied().unwrap_or(false);
            Provenance {
                tenant: scoped
                    && relation
                        .tenant_columns
                        .iter()
                        .any(|tenant| tenant == column),
                key_of: (scoped && relation.single_key() == Some(column)).then_some(*id),
            }
        }
        Source::Derived(derived) => derived
            .columns
            .iter()
            .find(|output| output.name == column)
            .map(|output| output.provenance)
            .unwrap_or_default(),
    }
}

fn direct(frames: &[Frame<'_>], catalog: &Catalog, operand: &Operand) -> Provenance {
    match operand {
        Operand::Parameter => Provenance::PARAMETER,
        Operand::Fixed(provenance) | Operand::List(provenance) => *provenance,
        Operand::Column { frame, rel, column } => own(frames, catalog, *frame, *rel, column),
        Operand::Other => Provenance::default(),
    }
}

/// What an operand is: a column is also whatever its level equates it to.
pub(super) fn provenance(frames: &[Frame<'_>], catalog: &Catalog, operand: &Operand) -> Provenance {
    match operand {
        Operand::Column { frame, rel, column } => sides(frames, *frame, *rel)
            .filter(|(mine, _)| mine == column)
            .fold(
                own(frames, catalog, *frame, *rel, column),
                |known, (_, other)| known.union(direct(frames, catalog, other)),
            ),
        other => direct(frames, catalog, other),
    }
}
