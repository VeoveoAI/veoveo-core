//! Exemption markers: SQL comments of the form `-- veoveo: <kind> (<reason>)`.
use pg_query::protobuf as pb;

use super::Rule;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MarkerKind {
    AllTenants,
    Unbounded,
    Scan,
}

impl MarkerKind {
    const ALL: [Self; 3] = [Self::AllTenants, Self::Unbounded, Self::Scan];

    pub fn spelling(self) -> &'static str {
        match self {
            Self::AllTenants => "all tenants",
            Self::Unbounded => "unbounded",
            Self::Scan => "scan",
        }
    }

    /// The check whose findings the marker exempts.
    pub fn rule(self) -> Rule {
        match self {
            Self::AllTenants => Rule::TenantPredicate,
            Self::Unbounded => Rule::RowBound,
            Self::Scan => Rule::IndexUse,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Marker {
    pub kind: MarkerKind,
    pub reason: String,
}

/// The markers of a query, and a description of every comment that starts like a
/// marker but is not one.
pub(super) fn parse(sql: &str) -> (Vec<Marker>, Vec<String>) {
    let mut markers = Vec::new();
    let mut problems = Vec::new();
    let Ok(scanned) = pg_query::scan(sql) else {
        return (markers, problems);
    };
    for token in scanned.tokens {
        if token.token != pb::Token::SqlComment as i32 {
            continue;
        }
        let Some(comment) = sql.get(token.start as usize..token.end as usize) else {
            continue;
        };
        let text = comment.trim_start_matches('-').trim();
        let Some(rest) = text.strip_prefix("veoveo:") else {
            continue;
        };
        match marker(rest.trim()) {
            Ok(marker) if markers.iter().any(|m: &Marker| m.kind == marker.kind) => {
                problems.push(format!("`{text}` repeats a marker"));
            }
            Ok(marker) => markers.push(marker),
            Err(problem) => problems.push(format!("`{text}` {problem}")),
        }
    }
    (markers, problems)
}

fn marker(text: &str) -> Result<Marker, String> {
    let kinds = || {
        MarkerKind::ALL
            .iter()
            .map(|kind| format!("`{}`", kind.spelling()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let Some(open) = text.find('(') else {
        return Err("needs a reason in parentheses, as `-- veoveo: <kind> (<reason>)`".to_owned());
    };
    let spelling = text[..open].trim();
    let Some(kind) = MarkerKind::ALL
        .into_iter()
        .find(|kind| kind.spelling() == spelling)
    else {
        return Err(format!("names no marker kind; the kinds are {}", kinds()));
    };
    let Some(reason) = text[open + 1..].trim_end().strip_suffix(')') else {
        return Err("needs a reason in parentheses, as `-- veoveo: <kind> (<reason>)`".to_owned());
    };
    let reason = reason.trim();
    if reason.is_empty() {
        return Err("needs a non-empty reason".to_owned());
    }
    Ok(Marker {
        kind,
        reason: reason.to_owned(),
    })
}
