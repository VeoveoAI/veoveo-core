//! Installation-owned roles, grants and overrides, and the registration settings. None
//! of these name a server: roles select permissions by access kind and risk, and
//! servers declare their own permissions when they register.
use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use veoveo_gateway_contract::SecretReferenceId;
use veoveo_mcp_registration_extension::{Access, Risk};
use veoveo_types::{GroupId, PermissionName, PrincipalId, RoleId, TenantId};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstallationAuthorization {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<InstallationRole>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<RoleGrant>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<PermissionOverride>,
}

/// Grants a permission when it allows the permission's access kind and every risk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstallationRole {
    pub name: RoleId,
    pub access: BTreeSet<Access>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub risks: BTreeSet<Risk>,
}

impl InstallationRole {
    pub fn allows(&self, access: Access, risks: &BTreeSet<Risk>) -> bool {
        self.access.contains(&access) && risks.is_subset(&self.risks)
    }
}

/// Gives one role to a group or principal, within one tenant or every tenant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleGrant {
    pub role: RoleId,
    pub subject: GrantSubject,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<TenantId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum GrantSubject {
    Group(GroupId),
    Principal(PrincipalId),
}

/// The escape hatch for a server that labels an action wrongly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
pub enum PermissionOverride {
    /// Treat the permission as carrying these additional risks.
    AddRisks {
        permission: PermissionName,
        risks: BTreeSet<Risk>,
    },
    /// Never grant the permission.
    Deny { permission: PermissionName },
}

impl PermissionOverride {
    pub fn permission(&self) -> &PermissionName {
        match self {
            Self::AddRisks { permission, .. } | Self::Deny { permission } => permission,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistrationSettings {
    /// A Gateway-owned secret with the `registration_secret` purpose.
    pub secret: SecretReferenceId,
    #[serde(default)]
    pub lease: LeaseTimings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaseTimings {
    pub renew_every_seconds: u32,
    pub expires_after_seconds: u32,
}

impl Default for LeaseTimings {
    fn default() -> Self {
        Self {
            renew_every_seconds: 10,
            expires_after_seconds: 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_sections_decode() {
        let authorization: InstallationAuthorization = serde_json::from_value(serde_json::json!({
            "roles": [
                { "name": "member", "access": ["read", "write"] },
                { "name": "admin", "access": ["read", "write"], "risks": ["sensitive", "destructive", "billed", "physical", "administer"] }
            ],
            "grants": [
                { "role": "member", "subject": { "group": "tenant-members" } },
                { "role": "admin", "subject": { "group": "installation-admins" } }
            ],
            "overrides": [{ "kind": "deny", "permission": "time.admin" }]
        }))
        .unwrap();
        assert_eq!(authorization.roles.len(), 2);
        assert!(matches!(
            &authorization.grants[0].subject,
            GrantSubject::Group(group) if group.as_str() == "tenant-members"
        ));
        assert_eq!(
            authorization.overrides[0].permission().as_str(),
            "time.admin"
        );
        let registration: RegistrationSettings = serde_json::from_value(serde_json::json!({
            "secret": "registration-secret",
            "lease": { "renewEverySeconds": 10, "expiresAfterSeconds": 30 }
        }))
        .unwrap();
        assert_eq!(registration.lease, LeaseTimings::default());
        let defaulted: RegistrationSettings =
            serde_json::from_value(serde_json::json!({ "secret": "registration-secret" })).unwrap();
        assert_eq!(defaulted.lease, LeaseTimings::default());
    }
}
