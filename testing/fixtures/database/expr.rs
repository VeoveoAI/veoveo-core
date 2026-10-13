//! Expression helpers over the PostgreSQL parse tree that `pg_query` returns.
use pg_query::{NodeEnum, protobuf as pb};

/// The arguments of a predicate's top-level `AND`s, or the predicate itself.
pub(super) fn conjuncts(node: &pb::Node) -> Vec<&pb::Node> {
    match &node.node {
        Some(NodeEnum::BoolExpr(expr)) if expr.boolop == pb::BoolExprType::AndExpr as i32 => {
            expr.args.iter().flat_map(conjuncts).collect()
        }
        _ => vec![node],
    }
}

/// Visit `node` and the expressions below it without entering subqueries. Returning
/// false from `visit` skips a node's children. Node kinds the walker does not know are
/// visited without children.
pub(super) fn walk<'a>(node: &'a pb::Node, visit: &mut impl FnMut(&'a NodeEnum) -> bool) {
    let Some(inner) = &node.node else {
        return;
    };
    if visit(inner) {
        for child in children(inner) {
            walk(child, visit);
        }
    }
}

fn children(inner: &NodeEnum) -> Vec<&pb::Node> {
    fn boxed(child: &Option<Box<pb::Node>>) -> impl Iterator<Item = &pb::Node> {
        child.as_deref().into_iter()
    }
    match inner {
        NodeEnum::AExpr(expr) => boxed(&expr.lexpr).chain(boxed(&expr.rexpr)).collect(),
        NodeEnum::BoolExpr(expr) => expr.args.iter().collect(),
        NodeEnum::FuncCall(call) => call
            .args
            .iter()
            .chain(&call.agg_order)
            .chain(boxed(&call.agg_filter))
            .collect(),
        NodeEnum::TypeCast(cast) => boxed(&cast.arg).collect(),
        NodeEnum::CollateClause(clause) => boxed(&clause.arg).collect(),
        NodeEnum::CoalesceExpr(expr) => expr.args.iter().collect(),
        NodeEnum::MinMaxExpr(expr) => expr.args.iter().collect(),
        NodeEnum::CaseExpr(expr) => boxed(&expr.arg)
            .chain(&expr.args)
            .chain(boxed(&expr.defresult))
            .collect(),
        NodeEnum::CaseWhen(when) => boxed(&when.expr).chain(boxed(&when.result)).collect(),
        NodeEnum::NullTest(test) => boxed(&test.arg).collect(),
        NodeEnum::BooleanTest(test) => boxed(&test.arg).collect(),
        NodeEnum::RowExpr(row) => row.args.iter().collect(),
        NodeEnum::AArrayExpr(array) => array.elements.iter().collect(),
        NodeEnum::AIndirection(indirection) => boxed(&indirection.arg)
            .chain(&indirection.indirection)
            .collect(),
        NodeEnum::AIndices(indices) => boxed(&indices.lidx).chain(boxed(&indices.uidx)).collect(),
        NodeEnum::ResTarget(target) => boxed(&target.val).collect(),
        NodeEnum::List(list) => list.items.iter().collect(),
        NodeEnum::NamedArgExpr(arg) => boxed(&arg.arg).collect(),
        NodeEnum::SortBy(sort) => boxed(&sort.node).collect(),
        NodeEnum::SubLink(link) => boxed(&link.testexpr).collect(),
        _ => Vec::new(),
    }
}

/// The subqueries that `node` contains outside other subqueries.
pub(super) fn sublinks<'a>(node: &'a pb::Node, found: &mut Vec<&'a pb::SubLink>) {
    walk(node, &mut |inner| {
        if let NodeEnum::SubLink(link) = inner {
            found.push(link);
        }
        true
    });
}

/// Whether `node` reads no column and no subquery: a parameter, a constant, or an
/// operator or function over them.
pub(super) fn is_fixed(node: &pb::Node) -> bool {
    let mut fixed = true;
    walk(node, &mut |inner| {
        let known = matches!(
            inner,
            NodeEnum::ParamRef(_)
                | NodeEnum::AConst(_)
                | NodeEnum::AExpr(_)
                | NodeEnum::BoolExpr(_)
                | NodeEnum::FuncCall(_)
                | NodeEnum::TypeCast(_)
                | NodeEnum::CollateClause(_)
                | NodeEnum::CoalesceExpr(_)
                | NodeEnum::MinMaxExpr(_)
                | NodeEnum::CaseExpr(_)
                | NodeEnum::CaseWhen(_)
                | NodeEnum::NullTest(_)
                | NodeEnum::BooleanTest(_)
                | NodeEnum::RowExpr(_)
                | NodeEnum::AArrayExpr(_)
                | NodeEnum::List(_)
                | NodeEnum::NamedArgExpr(_)
                | NodeEnum::SqlvalueFunction(_)
        );
        fixed &= known;
        known
    });
    fixed
}

/// Whether `node` is a bind parameter, possibly cast or collated.
pub(super) fn is_parameter(node: &pb::Node) -> bool {
    match &node.node {
        Some(NodeEnum::ParamRef(_)) => true,
        Some(NodeEnum::TypeCast(cast)) => cast.arg.as_deref().is_some_and(is_parameter),
        Some(NodeEnum::CollateClause(clause)) => clause.arg.as_deref().is_some_and(is_parameter),
        _ => false,
    }
}

/// The text of each `String` node in a name list, such as a function or operator name.
pub(super) fn names(list: &[pb::Node]) -> Vec<&str> {
    list.iter()
        .filter_map(|node| match &node.node {
            Some(NodeEnum::String(name)) => Some(name.sval.as_str()),
            _ => None,
        })
        .collect()
}

/// The last name of an operator, such as `=` for `OPERATOR(pg_catalog.=)`.
pub(super) fn operator(expr: &pb::AExpr) -> &str {
    names(&expr.name).last().copied().unwrap_or_default()
}

/// The schema and name of a called function.
pub(super) fn function_name(call: &pb::FuncCall) -> (Option<&str>, &str) {
    match names(&call.funcname).as_slice() {
        [schema, name] => (Some(schema), name),
        [.., name] => (None, name),
        [] => (None, ""),
    }
}

/// Whether a `LIMIT` or `FETCH FIRST` clause bounds the rows; `LIMIT ALL` and
/// `LIMIT NULL` do not.
pub(super) fn limits(count: Option<&pb::Node>) -> bool {
    match count.and_then(|node| node.node.as_ref()) {
        None => false,
        Some(NodeEnum::AConst(constant)) => !constant.isnull,
        Some(_) => true,
    }
}

/// The name a target list entry gives its output column, as PostgreSQL derives it.
pub(super) fn output_name(target: &pb::ResTarget) -> String {
    if !target.name.is_empty() {
        return target.name.clone();
    }
    let mut node = target.val.as_deref();
    while let Some(NodeEnum::TypeCast(cast)) = node.and_then(|node| node.node.as_ref()) {
        node = cast.arg.as_deref();
    }
    match node.and_then(|node| node.node.as_ref()) {
        Some(NodeEnum::ColumnRef(column)) => {
            match column.fields.last().and_then(|f| f.node.as_ref()) {
                Some(NodeEnum::String(name)) => name.sval.clone(),
                _ => "?column?".to_owned(),
            }
        }
        Some(NodeEnum::FuncCall(call)) => function_name(call).1.to_owned(),
        Some(NodeEnum::SubLink(link))
            if link.sub_link_type == pb::SubLinkType::ExistsSublink as i32 =>
        {
            "exists".to_owned()
        }
        _ => "?column?".to_owned(),
    }
}
