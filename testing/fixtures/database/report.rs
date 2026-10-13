//! The conformance report: every violation and every exemption, grouped by check.
use std::fmt;

/// The check a violation or an exemption belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    /// Every reference to a tenant table is constrained to one tenant.
    TenantPredicate,
    /// Every SELECT, UPDATE and DELETE touches a bounded number of rows.
    RowBound,
    /// The generic plan reaches every row of a checked table through an index.
    IndexUse,
    /// Tables have primary keys, indexed foreign keys and retention comments.
    Schema,
    /// Exemption markers are well formed and needed.
    Marker,
    /// The conformance configuration names relations the database has.
    Configuration,
    /// Statements parse, plan and name existing relations.
    Statement,
}

impl Rule {
    fn heading(self) -> &'static str {
        match self {
            Self::TenantPredicate => {
                "Tenant predicates (exempt a query with `-- veoveo: all tenants (<reason>)`)"
            }
            Self::RowBound => "Row bounds (exempt a query with `-- veoveo: unbounded (<reason>)`)",
            Self::IndexUse => "Index use (exempt a query with `-- veoveo: scan (<reason>)`)",
            Self::Schema => "Schema rules",
            Self::Marker => "Markers",
            Self::Configuration => "Configuration",
            Self::Statement => "Statements",
        }
    }

    /// The marker or configuration that exempts this check, as the report names it.
    fn exemption(self) -> &'static str {
        match self {
            Self::TenantPredicate => "all tenants",
            Self::RowBound => "unbounded",
            Self::IndexUse => "scan",
            _ => "exempt",
        }
    }
}

/// One broken rule. `subject` is the query's file relative to the workspace, or a
/// table, or the label of an inline query.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Violation {
    pub rule: Rule,
    pub subject: String,
    pub message: String,
}

/// One exemption a marker or the configuration granted, with its reason. A pending
/// exemption accepts violations as known work.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Exemption {
    pub rule: Rule,
    pub subject: String,
    pub reason: String,
    pub pending: bool,
}

/// The outcome of a conformance check.
#[derive(Debug, Clone)]
pub struct Report {
    pub queries: usize,
    pub tables: usize,
    pub schemas: Vec<String>,
    pub violations: Vec<Violation>,
    pub exemptions: Vec<Exemption>,
}

impl Report {
    pub fn is_conformant(&self) -> bool {
        self.violations.is_empty()
    }

    /// The violations of one check.
    pub fn violations_of(&self, rule: Rule) -> impl Iterator<Item = &Violation> {
        self.violations
            .iter()
            .filter(move |violation| violation.rule == rule)
    }
}

impl fmt::Display for Report {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let schemas = if self.schemas.is_empty() {
            "no schema".to_owned()
        } else {
            self.schemas.join(", ")
        };
        writeln!(
            formatter,
            "Database conformance: {} violation{} in {} queries and {} tables of {schemas}.",
            self.violations.len(),
            if self.violations.len() == 1 { "" } else { "s" },
            self.queries,
            self.tables,
        )?;
        let mut rule = None;
        let mut subject = None;
        for violation in &self.violations {
            if rule != Some(violation.rule) {
                writeln!(formatter, "\n{}", violation.rule.heading())?;
                rule = Some(violation.rule);
                subject = None;
            }
            if subject != Some(&violation.subject) {
                writeln!(formatter, "  {}", violation.subject)?;
                subject = Some(&violation.subject);
            }
            writeln!(formatter, "    {}.", violation.message)?;
        }
        if !self.exemptions.is_empty() {
            writeln!(formatter, "\nExemptions ({})", self.exemptions.len())?;
            for exemption in &self.exemptions {
                let kind = if exemption.pending {
                    "pending"
                } else {
                    exemption.rule.exemption()
                };
                writeln!(
                    formatter,
                    "  {kind}: {}: {}",
                    exemption.subject, exemption.reason
                )?;
            }
        }
        Ok(())
    }
}
