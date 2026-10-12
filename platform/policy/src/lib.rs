//! Shared pure authorization rules. Authentication and durable freshness are callers' responsibilities.
mod catalog;
mod evaluation;
mod grants;
pub mod session;
pub use catalog::{PolicyCatalog, PolicyCatalogView};
pub use evaluation::{
    PolicyRequest, RuleMatchDetail, RuleOutcome, assemble_rule_outcome, decide, intersects,
    mcp_method_name, principal_rule_conditions, remember_strongest_missing_requirement,
    resource_scheme_from_uri, strongest_missing_rule_detail,
};
pub use grants::{granted_permissions, held_roles};

pub mod internal_clients;
