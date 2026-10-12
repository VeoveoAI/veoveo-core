//! Decisions for registered servers' targets come from installation roles over the
//! server's declared permissions; the Gateway's own targets come from policy rules.
#[path = "../../../testing/fixtures/catalog_registry.rs"]
mod catalog_registry;

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use veoveo_gateway_contract::{
    AdmittedCatalogSections, AuthorizationServerId, CatalogFacts, CatalogRegistry, GatewayAction,
    PolicyAction, ProtectedResourceId,
};
use veoveo_mcp_contract::*;
use veoveo_mcp_ext_registration::{
    Access, AuthorizationBuilder, AuthorizationDeclaration, PermissionDefinition, Risk,
};
use veoveo_policy::{PolicyCatalogView, PolicyRequest, decide};
use veoveo_types::{
    DataLabelId, GroupId, PermissionName, PolicyVersion, PrincipalId, ResourceScheme, ResourceUri,
    RoleId, TenantId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, veoveo_types::Vocabulary)]
enum ClockPermission {
    #[vocabulary(rename = "clock.read")]
    Read,
    #[vocabulary(rename = "clock.events.write")]
    EventsWrite,
    #[vocabulary(rename = "clock.admin")]
    Admin,
}

impl PermissionDefinition for ClockPermission {
    fn resource(self) -> &'static str {
        match self {
            Self::EventsWrite => "clock.event",
            Self::Read | Self::Admin => "clock.authority",
        }
    }
    fn access(self) -> Access {
        match self {
            Self::Read => Access::Read,
            Self::EventsWrite | Self::Admin => Access::Write,
        }
    }
    fn risks(self) -> &'static [Risk] {
        match self {
            Self::Admin => &[Risk::Administer],
            _ => &[],
        }
    }
}

struct Catalog {
    registry: CatalogRegistry,
    sections: AdmittedCatalogSections,
    profile: GatewayProfile,
    policy: PolicySet,
    authorization: InstallationAuthorization,
    servers: BTreeMap<ServerSlug, AuthorizationDeclaration>,
}

impl PolicyCatalogView for Catalog {
    fn registry(&self) -> &CatalogRegistry {
        &self.registry
    }
    fn sections(&self) -> &AdmittedCatalogSections {
        &self.sections
    }
    fn profile(&self, id: &GatewayProfileId) -> Option<&GatewayProfile> {
        (id == &self.profile.id).then_some(&self.profile)
    }
    fn policy(&self, id: &PolicyVersion) -> Option<&PolicySet> {
        (id == &self.policy.version).then_some(&self.policy)
    }
    fn data_label(&self, _: &DataLabelId) -> Option<&DataLabelDefinition> {
        None
    }
    fn tenant(&self, _: &TenantId) -> Option<&TenantDefinition> {
        None
    }
    fn installation_authorization(&self) -> &InstallationAuthorization {
        &self.authorization
    }
    fn server_authorization(&self, server: &ServerSlug) -> Option<&AuthorizationDeclaration> {
        self.servers.get(server)
    }
}

fn clock() -> ServerSlug {
    ServerSlug::parse("clock").unwrap()
}

fn catalog() -> Catalog {
    let registry = catalog_registry::fresh_registry();
    let sections = registry
        .admit_sections(
            &BTreeMap::new(),
            &CatalogFacts {
                authorization_servers: BTreeSet::new(),
                policies: BTreeSet::new(),
                tenants: BTreeSet::new(),
                data_labels: BTreeSet::new(),
                oauth_clients: Vec::new(),
                work_contexts: Vec::new(),
            },
        )
        .unwrap();
    let declaration = AuthorizationBuilder::<ClockPermission>::new()
        .tool(
            LocalToolName::parse("resolve_time").unwrap(),
            ClockPermission::Read,
        )
        .tool(
            LocalToolName::parse("create_event").unwrap(),
            ClockPermission::EventsWrite,
        )
        .tool(
            LocalToolName::parse("activate_release").unwrap(),
            ClockPermission::Admin,
        )
        .resource_template(
            veoveo_types::ResourceTemplateUri::new("clock://events/{event}").unwrap(),
            ClockPermission::Read,
        )
        .build(&clock(), &ResourceScheme::parse("clock").unwrap())
        .unwrap();
    Catalog {
        registry,
        sections,
        profile: GatewayProfile {
            id: GatewayProfileId::parse("default").unwrap(),
            identity_provider: IdentityProviderId::parse("enterprise").unwrap(),
            authorization_server: AuthorizationServerId::parse("veoveo").unwrap(),
            protected_resource: ProtectedResourceId::parse("https://veoveo.example/mcp/default")
                .unwrap(),
            policy_version: PolicyVersion::parse("v1").unwrap(),
            artifact_upload: None,
            auth_modes: BTreeSet::from([AuthMode::OidcAuthorizationCodePkce]),
            discovery_failure_mode: DiscoveryFailureMode::Isolate,
            required_scopes: Vec::new(),
            metadata: Value::Null,
        },
        policy: PolicySet {
            version: PolicyVersion::parse("v1").unwrap(),
            rules: vec![PolicyRule {
                id: PolicyRuleId::parse("admins_read_admin").unwrap(),
                effect: PolicyEffect::Allow,
                actions: BTreeSet::from([GatewayAction::AdminRead.into()]),
                profiles: BTreeSet::new(),
                protected_resources: BTreeSet::new(),
                principal_ids: BTreeSet::new(),
                tenant_ids: BTreeSet::new(),
                groups: BTreeSet::from([GroupId::parse("installation-admins").unwrap()]),
                roles: BTreeSet::new(),
                required_scopes: BTreeSet::new(),
                required_data_labels: BTreeSet::new(),
                required_assurances: BTreeSet::new(),
                metadata: Value::Null,
            }],
            metadata: Value::Null,
        },
        authorization: InstallationAuthorization {
            roles: vec![
                InstallationRole {
                    name: RoleId::parse("member").unwrap(),
                    access: BTreeSet::from([Access::Read, Access::Write]),
                    risks: BTreeSet::new(),
                },
                InstallationRole {
                    name: RoleId::parse("viewer").unwrap(),
                    access: BTreeSet::from([Access::Read]),
                    risks: BTreeSet::new(),
                },
            ],
            grants: vec![
                RoleGrant {
                    role: RoleId::parse("member").unwrap(),
                    subject: GrantSubject::Group(GroupId::parse("members").unwrap()),
                    tenant: None,
                },
                RoleGrant {
                    role: RoleId::parse("viewer").unwrap(),
                    subject: GrantSubject::Group(GroupId::parse("viewers").unwrap()),
                    tenant: None,
                },
            ],
            overrides: Vec::new(),
        },
        servers: BTreeMap::from([(clock(), declaration)]),
    }
}

fn principal(group: &str) -> Principal {
    Principal {
        id: PrincipalId::parse("user:alice").unwrap(),
        kind: PrincipalKind::User,
        issuer: TokenIssuer::parse("https://idp.example").unwrap(),
        subject: TokenSubject::parse("alice").unwrap(),
        tenant: None,
        groups: BTreeSet::from([GroupId::parse(group).unwrap()]),
        group_roles: BTreeSet::new(),
        roles: BTreeSet::new(),
        scopes: BTreeSet::new(),
        data_labels: BTreeSet::new(),
        assurances: BTreeSet::new(),
        authenticated_at: None,
    }
}

fn decision(group: &str, action: GatewayAction, target: PolicyTarget) -> PolicyDecision {
    let principal = principal(group);
    decide(
        &catalog(),
        PolicyRequest {
            principal: &principal,
            profile: &GatewayProfileId::parse("default").unwrap(),
            action: PolicyAction::Kernel(action),
            target: &target,
            trace_id: &TraceId::parse("trace-1").unwrap(),
        },
    )
}

fn tool(name: &str) -> PolicyTarget {
    PolicyTarget::Tool {
        server: clock(),
        tool: LocalToolName::parse(name).unwrap(),
    }
}

#[test]
fn granted_roles_allow_declared_targets_and_forward_the_granted_set() {
    let allowed = decision("members", GatewayAction::ToolsCall, tool("create_event"));
    assert_eq!(allowed.effect, PolicyEffect::Allow);
    assert_eq!(
        allowed.permission,
        Some(PermissionName::parse("clock.events.write").unwrap())
    );
    assert_eq!(
        allowed.granted,
        BTreeSet::from([
            PermissionName::parse("clock.events.write").unwrap(),
            PermissionName::parse("clock.read").unwrap(),
        ])
    );
    let resource = decision(
        "viewers",
        GatewayAction::ResourcesRead,
        PolicyTarget::Resource {
            server: clock(),
            uri: ResourceUri::new("clock://events/abc").unwrap(),
        },
    );
    assert_eq!(resource.effect, PolicyEffect::Allow);
}

#[test]
fn missing_permissions_risks_and_declarations_deny() {
    let viewer = decision("viewers", GatewayAction::ToolsCall, tool("create_event"));
    assert_eq!(viewer.effect, PolicyEffect::Deny);
    assert_eq!(viewer.reason, PolicyReasonCode::MissingPermission);
    assert!(viewer.granted.is_empty());

    let admin_tool = decision(
        "members",
        GatewayAction::ToolsCall,
        tool("activate_release"),
    );
    assert_eq!(admin_tool.reason, PolicyReasonCode::MissingPermission);

    let undeclared = decision("members", GatewayAction::ToolsCall, tool("undeclared"));
    assert_eq!(undeclared.reason, PolicyReasonCode::UnknownTool);

    let other_resource = decision(
        "members",
        GatewayAction::ResourcesRead,
        PolicyTarget::Resource {
            server: clock(),
            uri: ResourceUri::new("clock://authorities/current").unwrap(),
        },
    );
    assert_eq!(other_resource.reason, PolicyReasonCode::UndeclaredTarget);

    let unregistered = decision(
        "members",
        GatewayAction::ToolsCall,
        PolicyTarget::Tool {
            server: ServerSlug::parse("map").unwrap(),
            tool: LocalToolName::parse("render").unwrap(),
        },
    );
    assert_eq!(unregistered.reason, PolicyReasonCode::UnknownServer);
}

#[test]
fn server_wide_operations_need_any_permission_in_the_namespace() {
    let listed = decision(
        "viewers",
        GatewayAction::ToolsList,
        PolicyTarget::Server { server: clock() },
    );
    assert_eq!(listed.effect, PolicyEffect::Allow);
    let stranger = decision(
        "strangers",
        GatewayAction::ToolsList,
        PolicyTarget::Server { server: clock() },
    );
    assert_eq!(stranger.reason, PolicyReasonCode::MissingPermission);
}

#[test]
fn gateway_targets_follow_policy_rules_and_ignore_roles() {
    let admin = decision(
        "installation-admins",
        GatewayAction::AdminRead,
        PolicyTarget::Gateway,
    );
    assert_eq!(admin.effect, PolicyEffect::Allow);
    let member = decision("members", GatewayAction::AdminRead, PolicyTarget::Gateway);
    assert_eq!(member.effect, PolicyEffect::Deny);
    // A rule-governed action never reaches a registered server's target.
    let crossed = decision(
        "installation-admins",
        GatewayAction::AdminRead,
        tool("resolve_time"),
    );
    assert_eq!(crossed.effect, PolicyEffect::Deny);
}
