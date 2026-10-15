//! The tenant-predicate and row-bound checks of one query, over the parse tree of
//! PostgreSQL's own parser.
use std::{collections::BTreeSet, ops::Range};

use pg_query::{NodeEnum, protobuf as pb};

use super::{
    Rule,
    catalog::{Catalog, RelationId},
    expr,
    scope::{self, Derived, Equality, Frame, Operand, OutputColumn, Provenance, Rel, Source},
};

/// The findings for one query.
pub(super) struct Analysis {
    pub findings: Vec<(Rule, String)>,
    /// The relations the query reads or writes.
    pub referenced: BTreeSet<RelationId>,
    /// The statements PostgreSQL can plan: SELECT, INSERT, UPDATE, DELETE and MERGE.
    pub plannable: Vec<String>,
}

pub(super) fn analyze(catalog: &Catalog, sql: &str) -> Result<Analysis, String> {
    let parsed = pg_query::parse(sql).map_err(|error| error.to_string())?;
    let texts = pg_query::split_with_parser(sql).map_err(|error| error.to_string())?;
    let mut analyzer = Analyzer {
        catalog,
        frames: Vec::new(),
        ctes: Vec::new(),
        quiet: 0,
        deepest: usize::MAX,
        findings: Vec::new(),
        referenced: BTreeSet::new(),
    };
    let mut plannable = Vec::new();
    for (raw, text) in parsed.protobuf.stmts.iter().zip(texts) {
        let Some(node) = raw.stmt.as_deref() else {
            continue;
        };
        match &node.node {
            Some(NodeEnum::SelectStmt(select)) => {
                let derived = analyzer.select(select);
                if !derived.bounded {
                    let reasons = if derived.unbounded.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", derived.unbounded.join(", "))
                    };
                    analyzer.report(
                        Rule::RowBound,
                        format!(
                            "the SELECT can return any number of rows{reasons}; add a LIMIT, \
                             pin a primary or unique key by equality, or aggregate without \
                             GROUP BY"
                        ),
                    );
                }
            }
            Some(
                NodeEnum::InsertStmt(_)
                | NodeEnum::UpdateStmt(_)
                | NodeEnum::DeleteStmt(_)
                | NodeEnum::MergeStmt(_),
            ) => {
                analyzer.statement(node);
            }
            _ => continue,
        }
        plannable.push(text.to_owned());
    }
    Ok(Analysis {
        findings: analyzer.findings,
        referenced: analyzer.referenced,
        plannable,
    })
}

struct Analyzer<'a> {
    catalog: &'a Catalog,
    frames: Vec<Frame<'a>>,
    ctes: Vec<(String, Derived)>,
    /// Above zero while a subquery is analyzed only for its bounds and columns.
    quiet: usize,
    /// The outermost query level a column reference resolved into.
    deepest: usize,
    findings: Vec<(Rule, String)>,
    referenced: BTreeSet<RelationId>,
}

impl<'a> Analyzer<'a> {
    fn report(&mut self, rule: Rule, message: String) {
        if self.quiet == 0
            && !self
                .findings
                .iter()
                .any(|(r, m)| *r == rule && *m == message)
        {
            self.findings.push((rule, message));
        }
    }

    fn top(&self) -> &Frame<'a> {
        self.frames.last().expect("an open query level")
    }

    fn top_mut(&mut self) -> &mut Frame<'a> {
        self.frames.last_mut().expect("an open query level")
    }

    /// Analyze a subquery for its result only, and say whether it read a column of an
    /// enclosing level.
    fn probe(&mut self, node: &'a pb::Node) -> (Derived, bool) {
        let depth = self.frames.len();
        let saved = std::mem::replace(&mut self.deepest, usize::MAX);
        self.quiet += 1;
        let derived = self.statement(node);
        self.quiet -= 1;
        let correlated = self.deepest < depth;
        self.deepest = saved.min(self.deepest);
        (derived, correlated)
    }

    fn statement(&mut self, node: &'a pb::Node) -> Derived {
        match &node.node {
            Some(NodeEnum::SelectStmt(statement)) => self.select(statement),
            Some(NodeEnum::InsertStmt(statement)) => self.insert(statement),
            Some(NodeEnum::UpdateStmt(statement)) => self.update(statement),
            Some(NodeEnum::DeleteStmt(statement)) => self.delete(statement),
            Some(NodeEnum::MergeStmt(statement)) => self.merge(statement),
            _ => Derived::default(),
        }
    }

    fn with(&mut self, clause: Option<&'a pb::WithClause>) -> usize {
        let mark = self.ctes.len();
        let Some(clause) = clause else {
            return mark;
        };
        for node in &clause.ctes {
            let Some(NodeEnum::CommonTableExpr(cte)) = &node.node else {
                continue;
            };
            if clause.recursive {
                let reason = format!("recursive WITH query {}", cte.ctename);
                self.ctes
                    .push((cte.ctename.clone(), Derived::unbounded(reason)));
            }
            let derived = cte
                .ctequery
                .as_deref()
                .map(|query| self.statement(query))
                .unwrap_or_default();
            if clause.recursive {
                self.ctes.pop();
            }
            let derived = rename(derived, &cte.aliascolnames);
            self.ctes.push((cte.ctename.clone(), derived));
        }
        mark
    }

    fn select(&mut self, statement: &'a pb::SelectStmt) -> Derived {
        let mark = self.with(statement.with_clause.as_ref());
        let limited = expr::limits(statement.limit_count.as_deref());
        let derived = if statement.op != pb::SetOperation::SetopNone as i32 {
            let left = statement
                .larg
                .as_deref()
                .map(|arm| self.select(arm))
                .unwrap_or_default();
            let right = statement
                .rarg
                .as_deref()
                .map(|arm| self.select(arm))
                .unwrap_or_default();
            let columns = left
                .columns
                .iter()
                .enumerate()
                .map(|(index, column)| OutputColumn {
                    name: column.name.clone(),
                    provenance: right
                        .columns
                        .get(index)
                        .map_or(Provenance::default(), |other| {
                            column.provenance.intersect(other.provenance)
                        }),
                })
                .collect();
            Derived {
                bounded: limited || (left.bounded && right.bounded),
                columns,
                unbounded: [left.unbounded, right.unbounded].concat(),
            }
        } else if let Some(first) = statement.values_lists.first() {
            let width = match &first.node {
                Some(NodeEnum::List(list)) => list.items.len(),
                _ => 0,
            };
            let columns = (0..width)
                .map(|index| OutputColumn {
                    name: format!("column{}", index + 1),
                    provenance: if statement.values_lists.iter().all(|row| match &row.node {
                        Some(NodeEnum::List(list)) => {
                            list.items.get(index).is_some_and(expr::is_parameter)
                        }
                        _ => false,
                    }) {
                        Provenance::PARAMETER
                    } else {
                        Provenance::default()
                    },
                })
                .collect();
            Derived::bounded(columns)
        } else {
            self.block(statement, limited)
        };
        self.ctes.truncate(mark);
        derived
    }

    fn block(&mut self, statement: &'a pb::SelectStmt, limited: bool) -> Derived {
        self.frames.push(Frame::default());
        for item in &statement.from_clause {
            self.range_item(item);
        }
        let all = 0..self.top().rels.len();
        if let Some(predicate) = statement.where_clause.as_deref() {
            self.predicate(predicate, all);
        }
        for target in &statement.target_list {
            self.collect_sublinks(target);
        }
        if let Some(having) = statement.having_clause.as_deref() {
            self.collect_sublinks(having);
        }
        self.solve(true);
        self.nested();
        let columns = self.outputs(&statement.target_list);
        let (aggregate, set_returning) = self.target_functions(&statement.target_list);
        let frame = self.frames.pop().expect("the block's level");
        let mut unbounded: Vec<String> = frame
            .rels
            .iter()
            .zip(&frame.bounded)
            .filter(|(_, bounded)| !**bounded)
            .map(|(rel, _)| format!("{} is not pinned by a primary or unique key", rel.label))
            .collect();
        if set_returning {
            unbounded.push("a set-returning function in the select list".to_owned());
        }
        let grouped = !statement.group_clause.is_empty();
        let bounded = limited
            || ((aggregate || statement.having_clause.is_some()) && !grouped)
            || unbounded.is_empty();
        Derived {
            bounded,
            columns,
            unbounded,
        }
    }

    fn insert(&mut self, statement: &'a pb::InsertStmt) -> Derived {
        let mark = self.with(statement.with_clause.as_ref());
        let source = statement
            .select_stmt
            .as_deref()
            .map(|source| self.statement(source));
        self.frames.push(Frame::default());
        if let Some(range) = &statement.relation {
            self.range_var(range);
        }
        let supplied: Vec<&str> = statement
            .cols
            .iter()
            .filter_map(|column| match &column.node {
                Some(NodeEnum::ResTarget(target)) => Some(target.name.as_str()),
                _ => None,
            })
            .collect();
        let mut supplies_tenant = false;
        if let Some(Rel {
            source: Source::Table(id),
            label,
            ..
        }) = self.top().rels.first().cloned()
        {
            let relation = self.catalog.relation(id);
            let required: Vec<&str> = relation
                .tenant_columns
                .iter()
                .filter(|tenant| {
                    !relation
                        .columns
                        .iter()
                        .any(|column| &column.name == *tenant && column.generated)
                })
                .map(String::as_str)
                .collect();
            supplies_tenant = supplied.iter().any(|column| required.contains(column));
            if !required.is_empty() && !supplies_tenant {
                self.report(
                    Rule::TenantPredicate,
                    format!(
                        "INSERT INTO {label} does not supply {}",
                        alternatives(&required)
                    ),
                );
            }
        }
        for target in &statement.returning_list {
            self.collect_sublinks(target);
        }
        self.solve(false);
        if let Some(constrained) = self.top_mut().constrained.first_mut() {
            *constrained = supplies_tenant;
        }
        self.nested();
        let columns = self.outputs(&statement.returning_list);
        self.frames.pop();
        self.ctes.truncate(mark);
        Derived {
            bounded: source.as_ref().is_none_or(|source| source.bounded),
            columns,
            unbounded: source.map(|source| source.unbounded).unwrap_or_default(),
        }
    }

    fn update(&mut self, statement: &'a pb::UpdateStmt) -> Derived {
        let mark = self.with(statement.with_clause.as_ref());
        self.frames.push(Frame::default());
        if let Some(range) = &statement.relation {
            self.range_var(range);
        }
        for item in &statement.from_clause {
            self.range_item(item);
        }
        let derived = self.modification(
            "UPDATE",
            statement.where_clause.as_deref(),
            &statement.target_list,
            &statement.returning_list,
        );
        self.ctes.truncate(mark);
        derived
    }

    fn delete(&mut self, statement: &'a pb::DeleteStmt) -> Derived {
        let mark = self.with(statement.with_clause.as_ref());
        self.frames.push(Frame::default());
        if let Some(range) = &statement.relation {
            self.range_var(range);
        }
        for item in &statement.using_clause {
            self.range_item(item);
        }
        let derived = self.modification(
            "DELETE",
            statement.where_clause.as_deref(),
            &[],
            &statement.returning_list,
        );
        self.ctes.truncate(mark);
        derived
    }

    fn merge(&mut self, statement: &'a pb::MergeStmt) -> Derived {
        let mark = self.with(statement.with_clause.as_ref());
        self.frames.push(Frame::default());
        if let Some(range) = &statement.relation {
            self.range_var(range);
        }
        if let Some(source) = statement.source_relation.as_deref() {
            self.range_item(source);
        }
        let derived = self.modification(
            "MERGE",
            statement.join_condition.as_deref(),
            &statement.merge_when_clauses,
            &statement.returning_list,
        );
        self.ctes.truncate(mark);
        derived
    }

    /// The rest of an UPDATE, DELETE or MERGE whose target and other relations are on
    /// the top level: its predicate, its tenant and row-bound checks and its output.
    fn modification(
        &mut self,
        verb: &str,
        predicate: Option<&'a pb::Node>,
        values: &'a [pb::Node],
        returning: &'a [pb::Node],
    ) -> Derived {
        let all = 0..self.top().rels.len();
        if let Some(predicate) = predicate {
            self.predicate(predicate, all);
        }
        for node in values.iter().chain(returning) {
            self.collect_sublinks(node);
        }
        self.solve(true);
        let target = self.top().rels.first().cloned();
        let keyed = self.top().bounded.first().copied().unwrap_or(false);
        if let Some(Rel {
            source: Source::Table(_),
            label,
            ..
        }) = target
            && !keyed
        {
            let message = if predicate.is_some() {
                format!(
                    "{verb} of {label} neither pins a primary or unique key nor selects its \
                     rows through a LIMIT-ed subquery"
                )
            } else {
                format!("{verb} of {label} has no WHERE clause")
            };
            self.report(Rule::RowBound, message);
        }
        self.nested();
        let columns = self.outputs(returning);
        self.frames.pop();
        if keyed {
            Derived::bounded(columns)
        } else {
            Derived {
                bounded: false,
                columns,
                unbounded: vec![format!("the {verb} target is not keyed")],
            }
        }
    }

    /// Solve the top level and, when `report`, name every tenant table it leaves
    /// unscoped.
    fn solve(&mut self, report: bool) {
        scope::solve(&mut self.frames, self.catalog);
        if !report {
            return;
        }
        let top = self.top();
        let unscoped: Vec<String> = top
            .rels
            .iter()
            .zip(&top.constrained)
            .filter_map(|(rel, constrained)| match rel.source {
                Source::Table(id) if !constrained => {
                    let tenants = &self.catalog.relation(id).tenant_columns;
                    (!tenants.is_empty()).then(|| {
                        let names: Vec<&str> = tenants.iter().map(String::as_str).collect();
                        format!(
                            "{} has no equality on {} to a bind parameter or to the tenant of \
                             a constrained relation",
                            rel.label,
                            alternatives(&names)
                        )
                    })
                }
                _ => None,
            })
            .collect();
        for message in unscoped {
            self.report(Rule::TenantPredicate, message);
        }
    }

    /// Analyze the subqueries of the solved top level, which may read its relations.
    fn nested(&mut self) {
        let sublinks = std::mem::take(&mut self.top_mut().sublinks);
        for link in sublinks {
            if let Some(query) = link.subselect.as_deref() {
                self.statement(query);
            }
        }
        let laterals = std::mem::take(&mut self.top_mut().laterals);
        for query in laterals {
            self.statement(query);
        }
    }

    fn push_rel(&mut self, rel: Rel) {
        self.top_mut().rels.push(rel);
    }

    fn range_item(&mut self, node: &'a pb::Node) -> Range<usize> {
        let start = self.top().rels.len();
        match &node.node {
            Some(NodeEnum::RangeVar(range)) => self.range_var(range),
            Some(NodeEnum::RangeSubselect(subselect)) => {
                if let Some(query) = subselect.subquery.as_deref() {
                    let derived = if subselect.lateral {
                        self.top_mut().laterals.push(query);
                        self.probe(query).0
                    } else {
                        self.statement(query)
                    };
                    let name = subselect
                        .alias
                        .as_ref()
                        .map_or_else(|| "subquery".to_owned(), |alias| alias.aliasname.clone());
                    let derived = match &subselect.alias {
                        Some(alias) => rename(derived, &alias.colnames),
                        None => derived,
                    };
                    self.push_rel(Rel {
                        label: format!("subquery `{name}`"),
                        name,
                        source: Source::Derived(derived),
                    });
                }
            }
            Some(NodeEnum::RangeFunction(function)) => self.range_function(function),
            Some(NodeEnum::JoinExpr(join)) => {
                let left = join
                    .larg
                    .as_deref()
                    .map_or(start..start, |node| self.range_item(node));
                let right = join
                    .rarg
                    .as_deref()
                    .map_or(left.end..left.end, |node| self.range_item(node));
                let scope = match pb::JoinType::try_from(join.jointype) {
                    Ok(pb::JoinType::JoinInner) => left.start..right.end,
                    Ok(pb::JoinType::JoinLeft) => right.clone(),
                    Ok(pb::JoinType::JoinRight) => left.clone(),
                    _ => right.end..right.end,
                };
                if let Some(quals) = join.quals.as_deref() {
                    self.predicate(quals, scope.clone());
                }
                let top = self.frames.len() - 1;
                for column in expr::names(&join.using_clause) {
                    let on_left = left
                        .clone()
                        .rev()
                        .find(|&rel| self.has_column(top, rel, column));
                    let on_right = right.clone().find(|&rel| self.has_column(top, rel, column));
                    if let (Some(l), Some(r)) = (on_left, on_right) {
                        let operand = |rel| Operand::Column {
                            frame: top,
                            rel,
                            column: column.to_owned(),
                        };
                        self.top_mut().equalities.push(Equality {
                            left: operand(l),
                            right: operand(r),
                            scope: scope.clone(),
                        });
                    }
                }
            }
            _ => {}
        }
        start..self.top().rels.len()
    }

    fn range_var(&mut self, range: &'a pb::RangeVar) {
        let name = range
            .alias
            .as_ref()
            .map_or_else(|| range.relname.clone(), |alias| alias.aliasname.clone());
        if range.schemaname.is_empty()
            && let Some((_, derived)) = self
                .ctes
                .iter()
                .rev()
                .find(|(cte, _)| *cte == range.relname)
        {
            let derived = match &range.alias {
                Some(alias) => rename(derived.clone(), &alias.colnames),
                None => derived.clone(),
            };
            self.push_rel(Rel {
                label: format!("WITH query `{name}`"),
                name,
                source: Source::Derived(derived),
            });
            return;
        }
        let schema = (!range.schemaname.is_empty()).then_some(range.schemaname.as_str());
        match self.catalog.resolve(schema, &range.relname) {
            Some(id) => {
                self.referenced.insert(id);
                let relation = self.catalog.relation(id);
                let label = if name == relation.name {
                    format!("`{}`", relation.qualified())
                } else {
                    format!("`{} AS {name}`", relation.qualified())
                };
                self.push_rel(Rel {
                    name,
                    source: Source::Table(id),
                    label,
                });
            }
            None => {
                let shown = match schema {
                    Some(schema) => format!("{schema}.{}", range.relname),
                    None => range.relname.clone(),
                };
                self.report(
                    Rule::Statement,
                    format!("relation `{shown}` does not exist in the migrated fixture database"),
                );
                self.push_rel(Rel {
                    label: format!("`{shown}`"),
                    name,
                    source: Source::Derived(Derived::unbounded(format!(
                        "unknown relation `{shown}`"
                    ))),
                });
            }
        }
    }

    fn range_function(&mut self, function: &'a pb::RangeFunction) {
        let mut bounded = true;
        let mut names = Vec::new();
        for item in &function.functions {
            let call = match &item.node {
                Some(NodeEnum::List(list)) => {
                    list.items.first().and_then(|node| match &node.node {
                        Some(NodeEnum::FuncCall(call)) => Some(call.as_ref()),
                        _ => None,
                    })
                }
                _ => None,
            };
            let Some(call) = call else {
                bounded = false;
                continue;
            };
            let (schema, name) = expr::function_name(call);
            let returns_set = self
                .catalog
                .function(schema, name)
                .is_none_or(|kind| kind.returns_set);
            bounded &= !returns_set || caller_bounded(name, call);
            names.push(name.to_owned());
        }
        let alias = function.alias.as_ref();
        let mut columns: Vec<String> = alias
            .map(|alias| {
                expr::names(&alias.colnames)
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if columns.is_empty() {
            columns = match (alias, names.as_slice()) {
                (Some(alias), [_]) => vec![alias.aliasname.clone()],
                _ => names.clone(),
            };
        }
        if function.ordinality {
            columns.push("ordinality".to_owned());
        }
        let name = alias.map_or_else(|| names.join(", "), |alias| alias.aliasname.clone());
        let columns = columns
            .into_iter()
            .map(|name| OutputColumn {
                name,
                provenance: Provenance::default(),
            })
            .collect();
        let derived = if bounded {
            Derived::bounded(columns)
        } else {
            Derived {
                bounded: false,
                columns,
                unbounded: vec![format!("set-returning function `{name}`")],
            }
        };
        self.push_rel(Rel {
            label: format!("function `{name}`"),
            name,
            source: Source::Derived(derived),
        });
    }

    fn collect_sublinks(&mut self, node: &'a pb::Node) {
        let mut found = Vec::new();
        expr::sublinks(node, &mut found);
        self.top_mut().sublinks.extend(found);
    }

    /// Record the subqueries of a predicate and the equalities of its conjuncts.
    fn predicate(&mut self, node: &'a pb::Node, scope: Range<usize>) {
        self.collect_sublinks(node);
        for conjunct in expr::conjuncts(node) {
            self.conjunct(conjunct, scope.clone());
        }
    }

    fn conjunct(&mut self, conjunct: &'a pb::Node, scope: Range<usize>) {
        match &conjunct.node {
            Some(NodeEnum::AExpr(expression)) => {
                let (Some(left), Some(right)) =
                    (expression.lexpr.as_deref(), expression.rexpr.as_deref())
                else {
                    return;
                };
                let kind = pb::AExprKind::try_from(expression.kind);
                let equals = expr::operator(expression) == "=";
                match kind {
                    Ok(pb::AExprKind::AexprOp) if equals => self.equal(left, right, scope),
                    Ok(pb::AExprKind::AexprNotDistinct) => self.equal(left, right, scope),
                    Ok(pb::AExprKind::AexprOpAny | pb::AExprKind::AexprIn) if equals => {
                        let right = self.list(right).map_or(Operand::Other, Operand::List);
                        let left = self.operand(left);
                        self.top_mut()
                            .equalities
                            .push(Equality { left, right, scope });
                    }
                    _ => {}
                }
            }
            Some(NodeEnum::SubLink(link))
                if link.sub_link_type == pb::SubLinkType::AnySublink as i32
                    && expr::names(&link.oper_name)
                        .last()
                        .is_none_or(|name| *name == "=") =>
            {
                let listed = link.subselect.as_deref().and_then(|query| {
                    let (derived, correlated) = self.probe(query);
                    (derived.bounded && !correlated).then_some(derived)
                });
                let tested: Vec<&'a pb::Node> = match link.testexpr.as_deref() {
                    Some(pb::Node {
                        node: Some(NodeEnum::RowExpr(row)),
                    }) => row.args.iter().collect(),
                    Some(node) => vec![node],
                    None => Vec::new(),
                };
                for (index, node) in tested.into_iter().enumerate() {
                    let left = self.operand(node);
                    let right = match &listed {
                        Some(derived) => Operand::List(
                            derived
                                .columns
                                .get(index)
                                .map(|column| column.provenance)
                                .unwrap_or_default(),
                        ),
                        None => Operand::Other,
                    };
                    self.top_mut().equalities.push(Equality {
                        left,
                        right,
                        scope: scope.clone(),
                    });
                }
            }
            Some(NodeEnum::NullTest(test))
                if test.nulltesttype == pb::NullTestType::IsNotNull as i32 =>
            {
                let top = self.frames.len() - 1;
                if let Some(Operand::Column { frame, rel, column }) =
                    test.arg.as_deref().map(|arg| self.operand(arg))
                    && frame == top
                {
                    self.top_mut().not_null.push((rel, column));
                }
            }
            _ => {}
        }
    }

    fn equal(&mut self, left: &'a pb::Node, right: &'a pb::Node, scope: Range<usize>) {
        if let (Some(NodeEnum::RowExpr(l)), Some(NodeEnum::RowExpr(r))) = (&left.node, &right.node)
            && l.args.len() == r.args.len()
        {
            for (left, right) in l.args.iter().zip(&r.args) {
                self.equal(left, right, scope.clone());
            }
            return;
        }
        let left = self.operand(left);
        let right = self.operand(right);
        self.top_mut()
            .equalities
            .push(Equality { left, right, scope });
    }

    /// What the values are when `= ANY (node)` or `IN (node)` compares with a bounded
    /// list of fixed values.
    fn list(&mut self, node: &'a pb::Node) -> Option<Provenance> {
        if expr::is_fixed(node) {
            return Some(Provenance::default());
        }
        match &node.node {
            Some(NodeEnum::TypeCast(cast)) => cast.arg.as_deref().and_then(|arg| self.list(arg)),
            Some(NodeEnum::SubLink(link))
                if link.sub_link_type == pb::SubLinkType::ArraySublink as i32 =>
            {
                link.subselect.as_deref().and_then(|query| {
                    let (derived, correlated) = self.probe(query);
                    (derived.bounded && !correlated).then(|| {
                        derived
                            .columns
                            .first()
                            .map(|column| column.provenance)
                            .unwrap_or_default()
                    })
                })
            }
            _ => None,
        }
    }

    fn operand(&mut self, node: &'a pb::Node) -> Operand {
        match &node.node {
            Some(NodeEnum::ParamRef(_)) => Operand::Parameter,
            Some(NodeEnum::TypeCast(cast)) => cast
                .arg
                .as_deref()
                .map_or(Operand::Other, |arg| self.operand(arg)),
            Some(NodeEnum::CollateClause(clause)) => clause
                .arg
                .as_deref()
                .map_or(Operand::Other, |arg| self.operand(arg)),
            Some(NodeEnum::ColumnRef(column)) => self.resolve(column).unwrap_or(Operand::Other),
            Some(NodeEnum::SubLink(link))
                if link.sub_link_type == pb::SubLinkType::ExprSublink as i32 =>
            {
                let Some(query) = link.subselect.as_deref() else {
                    return Operand::Other;
                };
                match self.probe(query) {
                    (_, true) => Operand::Other,
                    (derived, false) => Operand::Fixed(
                        derived
                            .columns
                            .first()
                            .map(|column| column.provenance)
                            .unwrap_or_default(),
                    ),
                }
            }
            _ if expr::is_fixed(node) => Operand::Fixed(Provenance::default()),
            _ => Operand::Other,
        }
    }

    fn has_column(&self, frame: usize, rel: usize, column: &str) -> bool {
        match &self.frames[frame].rels[rel].source {
            Source::Table(id) => self.catalog.relation(*id).has_column(column),
            Source::Derived(derived) => derived.columns.iter().any(|output| output.name == column),
        }
    }

    fn resolve(&mut self, reference: &pb::ColumnRef) -> Option<Operand> {
        let fields = expr::names(&reference.fields);
        if fields.len() != reference.fields.len() {
            return None;
        }
        let (schema, qualifier, column) = match fields.as_slice() {
            [column] => (None, None, *column),
            [qualifier, column] => (None, Some(*qualifier), *column),
            [schema, qualifier, column] => (Some(*schema), Some(*qualifier), *column),
            _ => return None,
        };
        for frame in (0..self.frames.len()).rev() {
            let rels = &self.frames[frame].rels;
            let found = match qualifier {
                None => {
                    let matching: Vec<usize> = (0..rels.len())
                        .filter(|&rel| self.has_column(frame, rel, column))
                        .collect();
                    match matching.as_slice() {
                        [] => None,
                        [rel] => Some(*rel),
                        _ => return None,
                    }
                }
                Some(qualifier) => rels.iter().rposition(|rel| {
                    rel.name == qualifier
                        && schema.is_none_or(|schema| match rel.source {
                            Source::Table(id) => self.catalog.relation(id).schema == schema,
                            Source::Derived(_) => false,
                        })
                }),
            };
            if let Some(rel) = found {
                self.deepest = self.deepest.min(frame);
                return Some(Operand::Column {
                    frame,
                    rel,
                    column: column.to_owned(),
                });
            }
        }
        None
    }

    /// The output columns of the top level's target or RETURNING list.
    fn outputs(&mut self, targets: &'a [pb::Node]) -> Vec<OutputColumn> {
        let top = self.frames.len() - 1;
        let mut columns = Vec::new();
        for node in targets {
            let Some(NodeEnum::ResTarget(target)) = &node.node else {
                continue;
            };
            let Some(value) = target.val.as_deref() else {
                continue;
            };
            if let Some(NodeEnum::ColumnRef(reference)) = &value.node
                && matches!(
                    reference
                        .fields
                        .last()
                        .and_then(|field| field.node.as_ref()),
                    Some(NodeEnum::AStar(_))
                )
            {
                let qualifier = expr::names(&reference.fields).first().copied();
                for rel in 0..self.top().rels.len() {
                    if qualifier.is_some_and(|qualifier| self.top().rels[rel].name != qualifier) {
                        continue;
                    }
                    for name in self.columns_of(top, rel) {
                        let operand = Operand::Column {
                            frame: top,
                            rel,
                            column: name.clone(),
                        };
                        columns.push(OutputColumn {
                            provenance: scope::provenance(&self.frames, self.catalog, &operand),
                            name,
                        });
                    }
                }
                continue;
            }
            let operand = self.operand(value);
            columns.push(OutputColumn {
                name: expr::output_name(target),
                provenance: scope::provenance(&self.frames, self.catalog, &operand),
            });
        }
        columns
    }

    fn columns_of(&self, frame: usize, rel: usize) -> Vec<String> {
        match &self.frames[frame].rels[rel].source {
            Source::Table(id) => self
                .catalog
                .relation(*id)
                .columns
                .iter()
                .map(|column| column.name.clone())
                .collect(),
            Source::Derived(derived) => derived
                .columns
                .iter()
                .map(|column| column.name.clone())
                .collect(),
        }
    }

    /// Whether the target list aggregates, and whether it returns a set.
    fn target_functions(&self, targets: &[pb::Node]) -> (bool, bool) {
        let (mut aggregate, mut set_returning) = (false, false);
        for target in targets {
            expr::walk(target, &mut |node| {
                if let NodeEnum::FuncCall(call) = node
                    && call.over.is_none()
                {
                    let (schema, name) = expr::function_name(call);
                    let kind = self.catalog.function(schema, name).unwrap_or_default();
                    aggregate |= call.agg_star || kind.aggregate;
                    set_returning |= kind.returns_set && !caller_bounded(name, call);
                }
                true
            });
        }
        (aggregate, set_returning)
    }
}

/// `unnest` over parameters and constants returns as many rows as the caller passed.
fn caller_bounded(name: &str, call: &pb::FuncCall) -> bool {
    name == "unnest" && call.args.iter().all(expr::is_fixed)
}

fn rename(mut derived: Derived, names: &[pb::Node]) -> Derived {
    for (column, name) in derived.columns.iter_mut().zip(expr::names(names)) {
        column.name = name.to_owned();
    }
    derived
}

/// `a`, `a` or `b`, `a`, `b` or `c`.
fn alternatives(names: &[&str]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    match quoted.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}
