//! Resolved invocation authority, independent of protocol transport.
//! Values carry claims; authentication and policy enforcement establish their authority.
use crate::{InvocationProvenance, PolicyVersion, TenantId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A capability level. Ordered `Read < Write < Admin`, so `min` yields the
/// lesser privilege — exactly what capping a group role by a grant level needs.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AccessLevel {
    Read,
    Write,
    Admin,
}

impl AccessLevel {
    /// True when `self` is sufficient for a request that needs `required`.
    pub fn allows(self, required: AccessLevel) -> bool {
        self >= required
    }
}

/// Invocation authority resolved at admission and signed into every forwarded identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct InvocationAuthority {
    pub tenant: TenantId,
    pub policy_revision: PolicyVersion,
    pub provenance: InvocationProvenance,
}
