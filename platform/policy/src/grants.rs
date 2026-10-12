//! Role evaluation over registered servers' declared permissions. Pure: no I/O, no
//! clock, no server names. Installation roles select permissions by access kind and
//! risk; grants give roles to groups and principals within a tenant.
use std::collections::BTreeSet;

use veoveo_mcp_contract::{GrantSubject, InstallationAuthorization, PermissionOverride, Principal};
use veoveo_mcp_ext_registration::{AuthorizationDeclaration, PermissionDeclaration, Risk};
use veoveo_types::{PermissionName, RoleId};

/// The installation roles `principal` holds: grants to the principal itself or to one of
/// its groups, in its tenant or in every tenant.
pub fn held_roles(
    installation: &InstallationAuthorization,
    principal: &Principal,
) -> BTreeSet<RoleId> {
    installation
        .grants
        .iter()
        .filter(|grant| {
            grant
                .tenant
                .as_ref()
                .is_none_or(|tenant| principal.tenant.as_ref() == Some(tenant))
        })
        .filter(|grant| match &grant.subject {
            GrantSubject::Group(group) => principal.groups.contains(group),
            GrantSubject::Principal(id) => &principal.id == id,
        })
        .map(|grant| grant.role.clone())
        .collect()
}

/// Every permission of one server's declaration that `principal` holds. A permission
/// is granted when some held role allows its access kind and every one of its risks,
/// after installation overrides; a denied permission is never granted.
pub fn granted_permissions(
    installation: &InstallationAuthorization,
    declaration: &AuthorizationDeclaration,
    principal: &Principal,
) -> BTreeSet<PermissionName> {
    let held = held_roles(installation, principal);
    let roles: Vec<_> = installation
        .roles
        .iter()
        .filter(|role| held.contains(&role.name))
        .collect();
    declaration
        .permissions
        .iter()
        .filter(|permission| {
            let Some(risks) = effective_risks(installation, permission) else {
                return false;
            };
            roles
                .iter()
                .any(|role| role.allows(permission.access, &risks))
        })
        .map(|permission| permission.permission.clone())
        .collect()
}

/// Declared risks plus installation additions; `None` when the installation denies it.
fn effective_risks(
    installation: &InstallationAuthorization,
    permission: &PermissionDeclaration,
) -> Option<BTreeSet<Risk>> {
    let mut risks = permission.risks.clone();
    for permission_override in &installation.overrides {
        if permission_override.permission() != &permission.permission {
            continue;
        }
        match permission_override {
            PermissionOverride::Deny { .. } => return None,
            PermissionOverride::AddRisks { risks: added, .. } => risks.extend(added),
        }
    }
    Some(risks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use veoveo_mcp_contract::{InstallationRole, PrincipalKind, RoleGrant};
    use veoveo_mcp_ext_registration::{Access, PermissionDeclaration, Requirements};
    use veoveo_types::{GroupId, PrincipalId, ResourceTypeName, TenantId};

    fn permission(name: &str, access: Access, risks: &[Risk]) -> PermissionDeclaration {
        PermissionDeclaration {
            permission: PermissionName::parse(name).unwrap(),
            resource: ResourceTypeName::parse("time.authority").unwrap(),
            access,
            risks: risks.iter().copied().collect(),
        }
    }

    fn declaration() -> AuthorizationDeclaration {
        AuthorizationDeclaration {
            permissions: vec![
                permission("time.read", Access::Read, &[]),
                permission("time.events.write", Access::Write, &[]),
                permission("time.admin", Access::Write, &[Risk::Administer]),
                permission("time.exports.read", Access::Read, &[Risk::Sensitive]),
            ],
            requirements: Requirements::default(),
        }
    }

    fn role(name: &str, access: &[Access], risks: &[Risk]) -> InstallationRole {
        InstallationRole {
            name: RoleId::parse(name).unwrap(),
            access: access.iter().copied().collect(),
            risks: risks.iter().copied().collect(),
        }
    }

    fn grant(role: &str, group: &str, tenant: Option<&str>) -> RoleGrant {
        RoleGrant {
            role: RoleId::parse(role).unwrap(),
            subject: GrantSubject::Group(GroupId::parse(group).unwrap()),
            tenant: tenant.map(|tenant| TenantId::parse(tenant).unwrap()),
        }
    }

    fn installation() -> InstallationAuthorization {
        InstallationAuthorization {
            roles: vec![
                role("member", &[Access::Read, Access::Write], &[]),
                role(
                    "admin",
                    &[Access::Read, Access::Write],
                    &[Risk::Sensitive, Risk::Administer],
                ),
            ],
            grants: vec![
                grant("member", "tenant-members", Some("dev")),
                grant("admin", "installation-admins", None),
            ],
            overrides: Vec::new(),
        }
    }

    fn principal(groups: &[&str], tenant: &str) -> Principal {
        Principal {
            id: PrincipalId::parse("user:alice").unwrap(),
            kind: PrincipalKind::User,
            issuer: veoveo_types::TokenIssuer::parse("https://idp.example").unwrap(),
            subject: veoveo_types::TokenSubject::parse("alice").unwrap(),
            tenant: Some(TenantId::parse(tenant).unwrap()),
            groups: groups.iter().map(|g| GroupId::parse(*g).unwrap()).collect(),
            group_roles: BTreeSet::new(),
            roles: BTreeSet::new(),
            scopes: BTreeSet::new(),
            data_labels: BTreeSet::new(),
            assurances: BTreeSet::new(),
            authenticated_at: None,
        }
    }

    fn names(set: BTreeSet<PermissionName>) -> Vec<String> {
        set.into_iter()
            .map(|name| name.as_str().to_owned())
            .collect()
    }

    #[test]
    fn a_role_grants_only_permissions_whose_risks_it_allows() {
        let member = principal(&["tenant-members"], "dev");
        assert_eq!(
            names(granted_permissions(
                &installation(),
                &declaration(),
                &member
            )),
            ["time.events.write", "time.read"]
        );
        let admin = principal(&["installation-admins"], "other");
        assert_eq!(
            names(granted_permissions(&installation(), &declaration(), &admin)),
            [
                "time.admin",
                "time.events.write",
                "time.exports.read",
                "time.read"
            ]
        );
    }

    #[test]
    fn grants_are_scoped_to_their_tenant() {
        let member_elsewhere = principal(&["tenant-members"], "other");
        assert!(granted_permissions(&installation(), &declaration(), &member_elsewhere).is_empty());
        let nobody = principal(&[], "dev");
        assert!(granted_permissions(&installation(), &declaration(), &nobody).is_empty());
    }

    #[test]
    fn a_risk_no_role_names_fails_closed() {
        let mut declaration = declaration();
        declaration.permissions.push(permission(
            "time.drones.launch",
            Access::Write,
            &[Risk::Physical],
        ));
        let admin = principal(&["installation-admins"], "dev");
        assert!(
            !granted_permissions(&installation(), &declaration, &admin)
                .contains(&PermissionName::parse("time.drones.launch").unwrap())
        );
    }

    #[test]
    fn overrides_deny_or_add_risks() {
        let member = principal(&["tenant-members"], "dev");
        let mut installation = installation();
        installation.overrides = vec![
            PermissionOverride::Deny {
                permission: PermissionName::parse("time.read").unwrap(),
            },
            PermissionOverride::AddRisks {
                permission: PermissionName::parse("time.events.write").unwrap(),
                risks: BTreeSet::from([Risk::Destructive]),
            },
        ];
        assert!(granted_permissions(&installation, &declaration(), &member).is_empty());
        let admin = principal(&["installation-admins"], "dev");
        let granted = names(granted_permissions(&installation, &declaration(), &admin));
        assert!(!granted.contains(&"time.read".to_owned()));
        // Admins lack the destructive risk, so the added risk removes the grant too.
        assert!(!granted.contains(&"time.events.write".to_owned()));
        assert!(granted.contains(&"time.admin".to_owned()));
    }

    #[test]
    fn grants_to_one_principal_apply_only_to_it() {
        let mut installation = installation();
        installation.grants = vec![RoleGrant {
            role: RoleId::parse("member").unwrap(),
            subject: GrantSubject::Principal(PrincipalId::parse("user:alice").unwrap()),
            tenant: None,
        }];
        let alice = principal(&[], "dev");
        assert_eq!(
            names(granted_permissions(&installation, &declaration(), &alice)),
            ["time.events.write", "time.read"]
        );
        let mut bob = principal(&[], "dev");
        bob.id = PrincipalId::parse("user:bob").unwrap();
        assert!(granted_permissions(&installation, &declaration(), &bob).is_empty());
    }
}
