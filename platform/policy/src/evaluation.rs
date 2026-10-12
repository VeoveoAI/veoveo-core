use std::collections::BTreeSet;
use veoveo_gateway_contract::{GatewayAction, PolicyAction};

use anyhow::Result;
use veoveo_mcp_contract::{
    GatewayProfile, GatewayProfileId, McpMethodName, PolicyDecision, PolicyEffect,
    PolicyReasonCode, PolicyRule, PolicyRuleId, PolicyTarget, Principal, TraceId,
};
use veoveo_mcp_ext_registration::RequirementTarget;
use veoveo_types::{PermissionName, PolicyVersion, ResourceScheme, ScopeName};

use crate::PolicyCatalogView;

#[derive(Debug, Clone)]
pub struct PolicyRequest<'a> {
    pub principal: &'a Principal,
    pub profile: &'a GatewayProfileId,
    pub action: PolicyAction,
    pub target: &'a PolicyTarget,
    pub trace_id: &'a TraceId,
}

pub fn mcp_method_name(action: GatewayAction) -> Result<McpMethodName> {
    let Some(method) = action.mcp_method() else {
        anyhow::bail!("gateway action {action:?} does not map to one MCP method")
    };
    Ok(McpMethodName::parse(method)?)
}

pub fn resource_scheme_from_uri(uri: &str) -> Option<ResourceScheme> {
    let (scheme, _) = uri.split_once("://")?;
    ResourceScheme::parse(scheme).ok()
}

/// Registered servers' targets are decided by installation roles over the server's
/// declared permissions. The Gateway's own targets are decided by policy rules.
pub fn decide(catalog: &impl PolicyCatalogView, request: PolicyRequest<'_>) -> PolicyDecision {
    if validate_runtime_action(catalog, &request.action, request.target).is_err() {
        return deny(&request, PolicyReasonCode::PolicyDeny, None);
    }
    let Some(profile) = catalog.profile(request.profile) else {
        return deny(&request, PolicyReasonCode::UnknownProfile, None);
    };
    let Some(policy) = catalog.policy(&profile.policy_version) else {
        return deny(&request, PolicyReasonCode::PolicyDeny, None);
    };
    let version = Some(policy.version.clone());
    if let Err(reason) = validate_principal(catalog, request.principal) {
        return deny(&request, reason, version);
    }
    if !has_required_scopes(&request.principal.scopes, &profile.required_scopes) {
        return deny(&request, PolicyReasonCode::MissingScope, version);
    }
    match registered_target(request.target) {
        Some((server, requirement)) => {
            decide_server(catalog, &request, server, requirement, version)
        }
        None => decide_by_rules(profile, policy, &request, version),
    }
}

/// The server a target belongs to, and what it requires: a specific declared target,
/// or any permission in the server's namespace for server-wide operations and Tasks.
fn registered_target(
    target: &PolicyTarget,
) -> Option<(&veoveo_types::ServerSlug, Option<RequirementTarget<'_>>)> {
    match target {
        PolicyTarget::Server { server }
        | PolicyTarget::Task { server, .. }
        | PolicyTarget::PlatformTask { server, .. } => Some((server, None)),
        PolicyTarget::Tool { server, tool } => Some((server, Some(RequirementTarget::Tool(tool)))),
        PolicyTarget::Resource { server, uri } => {
            Some((server, Some(RequirementTarget::Resource(uri))))
        }
        PolicyTarget::ResourceTemplate { server, uri } => {
            Some((server, Some(RequirementTarget::ResourceTemplate(uri))))
        }
        PolicyTarget::Prompt { server, prompt } => {
            Some((server, Some(RequirementTarget::Prompt(prompt))))
        }
        PolicyTarget::Gateway
        | PolicyTarget::Artifact { .. }
        | PolicyTarget::Usage { .. }
        | PolicyTarget::Owner(_)
        | PolicyTarget::Unadmitted(_) => None,
    }
}

fn decide_server(
    catalog: &impl PolicyCatalogView,
    request: &PolicyRequest<'_>,
    server: &veoveo_types::ServerSlug,
    requirement: Option<RequirementTarget<'_>>,
    version: Option<PolicyVersion>,
) -> PolicyDecision {
    if !matches!(&request.action, PolicyAction::Kernel(action) if action.targets_registered_server())
    {
        return deny(request, PolicyReasonCode::PolicyDeny, version);
    }
    let Some(declaration) = catalog.server_authorization(server) else {
        return deny(request, PolicyReasonCode::UnknownServer, version);
    };
    let granted = crate::granted_permissions(
        catalog.installation_authorization(),
        declaration,
        request.principal,
    );
    let required = match requirement {
        Some(target) => match declaration.required(target) {
            Some(required) => Some(required.permission.clone()),
            None => {
                let reason = match target {
                    RequirementTarget::Tool(_) => PolicyReasonCode::UnknownTool,
                    RequirementTarget::Prompt(_) => PolicyReasonCode::UnknownPrompt,
                    RequirementTarget::Resource(_) | RequirementTarget::ResourceTemplate(_) => {
                        PolicyReasonCode::UndeclaredTarget
                    }
                };
                return deny(request, reason, version);
            }
        },
        None => None,
    };
    let allowed = match &required {
        Some(permission) => granted.contains(permission),
        None => !granted.is_empty(),
    };
    let mut decision = decision(
        request,
        if allowed {
            PolicyEffect::Allow
        } else {
            PolicyEffect::Deny
        },
        if allowed {
            PolicyReasonCode::PolicyAllow
        } else {
            PolicyReasonCode::MissingPermission
        },
        version,
        None,
    );
    decision.permission = required;
    if allowed {
        decision.granted = granted;
    }
    decision
}

fn decide_by_rules(
    profile: &GatewayProfile,
    policy: &veoveo_mcp_contract::PolicySet,
    request: &PolicyRequest<'_>,
    version: Option<PolicyVersion>,
) -> PolicyDecision {
    if matches!(
        request.target,
        PolicyTarget::Owner(_) | PolicyTarget::Unadmitted(_)
    ) {
        return deny(request, PolicyReasonCode::PolicyDeny, version);
    }
    let outcome = evaluate_rules(profile, policy, request.principal, &request.action.name());
    decision(
        request,
        outcome.effect,
        outcome.reason,
        version,
        outcome.rule_id,
    )
}

pub(crate) fn validate_principal(
    catalog: &impl PolicyCatalogView,
    principal: &Principal,
) -> Result<(), PolicyReasonCode> {
    if principal
        .data_labels
        .iter()
        .any(|label| catalog.data_label(label).is_none())
    {
        return Err(PolicyReasonCode::UnknownDataLabel);
    }
    if principal
        .tenant
        .as_ref()
        .is_some_and(|tenant| catalog.tenant(tenant).is_none())
    {
        return Err(PolicyReasonCode::UnknownTenant);
    }
    Ok(())
}

pub struct RuleOutcome {
    pub effect: PolicyEffect,
    pub reason: PolicyReasonCode,
    pub rule_id: Option<PolicyRuleId>,
}

/// One rule evaluator for concrete requests and resource-family admission.
/// The latter defers only lexical profile selection to the database.
pub(crate) fn evaluate_rules(
    profile: &GatewayProfile,
    policy: &veoveo_mcp_contract::PolicySet,
    principal: &Principal,
    action: &veoveo_types::ActionName,
) -> RuleOutcome {
    let detail = |rule: &PolicyRule| rule_match_detail(rule, profile, principal, action);
    assemble_rule_outcome(policy, detail)
}

/// Deny precedence and missing-condition diagnostics shared by every owner adapter.
pub fn assemble_rule_outcome(
    policy: &veoveo_mcp_contract::PolicySet,
    detail: impl Fn(&PolicyRule) -> RuleMatchDetail,
) -> RuleOutcome {
    if let Some(rule) = policy
        .rules
        .iter()
        .find(|rule| rule.effect == PolicyEffect::Deny && detail(rule) == RuleMatchDetail::Match)
    {
        return RuleOutcome {
            effect: PolicyEffect::Deny,
            reason: PolicyReasonCode::PolicyDeny,
            rule_id: Some(rule.id.clone()),
        };
    }
    let mut missing = None;
    for rule in &policy.rules {
        if rule.effect != PolicyEffect::Allow {
            continue;
        }
        let reason = match detail(rule) {
            RuleMatchDetail::Match => {
                return RuleOutcome {
                    effect: PolicyEffect::Allow,
                    reason: PolicyReasonCode::PolicyAllow,
                    rule_id: Some(rule.id.clone()),
                };
            }
            RuleMatchDetail::MissingDataLabel => PolicyReasonCode::MissingDataLabel,
            RuleMatchDetail::MissingPrincipalAssurance => {
                PolicyReasonCode::MissingPrincipalAssurance
            }
            RuleMatchDetail::MissingRole => PolicyReasonCode::MissingRole,
            RuleMatchDetail::MissingGroup => PolicyReasonCode::MissingGroup,
            RuleMatchDetail::MissingTenant => PolicyReasonCode::MissingTenant,
            RuleMatchDetail::MissingPrincipal => PolicyReasonCode::MissingPrincipal,
            RuleMatchDetail::MissingScope => PolicyReasonCode::MissingScope,
            RuleMatchDetail::NoMatch => continue,
        };
        remember_strongest_missing_requirement(&mut missing, reason, rule.id.clone());
    }
    let (reason, rule_id) = match missing {
        Some((reason, id)) => (reason, Some(id)),
        None => (PolicyReasonCode::PolicyDeny, None),
    };
    RuleOutcome {
        effect: PolicyEffect::Deny,
        reason,
        rule_id,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleMatchDetail {
    Match,
    MissingPrincipal,
    MissingTenant,
    MissingGroup,
    MissingRole,
    MissingScope,
    MissingDataLabel,
    MissingPrincipalAssurance,
    NoMatch,
}

fn deny(
    request: &PolicyRequest<'_>,
    reason: PolicyReasonCode,
    policy_version: Option<PolicyVersion>,
) -> PolicyDecision {
    decision(request, PolicyEffect::Deny, reason, policy_version, None)
}

fn decision(
    request: &PolicyRequest<'_>,
    effect: PolicyEffect,
    reason: PolicyReasonCode,
    policy_version: Option<PolicyVersion>,
    rule_id: Option<veoveo_mcp_contract::PolicyRuleId>,
) -> PolicyDecision {
    PolicyDecision {
        effect,
        reason,
        evaluated_at: chrono::Utc::now(),
        profile: request.profile.clone(),
        action: request.action.name(),
        target: request.target.clone(),
        principal: Some(request.principal.id.clone()),
        tenant: request.principal.tenant.clone(),
        policy_version,
        rule_id,
        permission: None,
        granted: BTreeSet::<PermissionName>::new(),
        trace_id: request.trace_id.clone(),
    }
}

fn rule_match_detail(
    rule: &PolicyRule,
    profile: &GatewayProfile,
    principal: &Principal,
    action: &veoveo_types::ActionName,
) -> RuleMatchDetail {
    if !rule.actions.contains(action) {
        return RuleMatchDetail::NoMatch;
    }
    if !rule.profiles.is_empty() && !rule.profiles.contains(&profile.id) {
        return RuleMatchDetail::NoMatch;
    }
    principal_rule_conditions(rule, principal, &principal.data_labels)
}

/// Labels are supplied by the owning authority (principal labels or producer labels).
pub fn principal_rule_conditions(
    rule: &PolicyRule,
    principal: &Principal,
    labels: &BTreeSet<veoveo_types::DataLabelId>,
) -> RuleMatchDetail {
    let mut strongest_missing_requirement = RuleMatchDetail::Match;
    if !rule.principal_ids.is_empty() && !rule.principal_ids.contains(&principal.id) {
        strongest_missing_requirement = strongest_missing_rule_detail(
            strongest_missing_requirement,
            RuleMatchDetail::MissingPrincipal,
        );
    }
    if !rule.tenant_ids.is_empty() {
        match &principal.tenant {
            Some(tenant) if rule.tenant_ids.contains(tenant) => {}
            _ => {
                strongest_missing_requirement = strongest_missing_rule_detail(
                    strongest_missing_requirement,
                    RuleMatchDetail::MissingTenant,
                );
            }
        }
    }
    if !rule.groups.is_empty() && !intersects(&rule.groups, &principal.groups) {
        strongest_missing_requirement = strongest_missing_rule_detail(
            strongest_missing_requirement,
            RuleMatchDetail::MissingGroup,
        );
    }
    if !rule.roles.is_empty() && !intersects(&rule.roles, &principal.roles) {
        strongest_missing_requirement = strongest_missing_rule_detail(
            strongest_missing_requirement,
            RuleMatchDetail::MissingRole,
        );
    }
    if !rule.required_scopes.is_empty() && !rule.required_scopes.is_subset(&principal.scopes) {
        strongest_missing_requirement = strongest_missing_rule_detail(
            strongest_missing_requirement,
            RuleMatchDetail::MissingScope,
        );
    }
    if !rule.required_data_labels.is_empty() && !rule.required_data_labels.is_subset(labels) {
        strongest_missing_requirement = strongest_missing_rule_detail(
            strongest_missing_requirement,
            RuleMatchDetail::MissingDataLabel,
        );
    }
    if !rule.required_assurances.is_empty()
        && !rule.required_assurances.is_subset(&principal.assurances)
    {
        strongest_missing_requirement = strongest_missing_rule_detail(
            strongest_missing_requirement,
            RuleMatchDetail::MissingPrincipalAssurance,
        );
    }
    strongest_missing_requirement
}

pub fn strongest_missing_rule_detail(
    left: RuleMatchDetail,
    right: RuleMatchDetail,
) -> RuleMatchDetail {
    if rule_detail_rank(right) > rule_detail_rank(left) {
        right
    } else {
        left
    }
}

fn rule_detail_rank(detail: RuleMatchDetail) -> u8 {
    match detail {
        RuleMatchDetail::MissingDataLabel => 70,
        RuleMatchDetail::MissingPrincipalAssurance => 60,
        RuleMatchDetail::MissingRole => 50,
        RuleMatchDetail::MissingGroup => 40,
        RuleMatchDetail::MissingTenant => 30,
        RuleMatchDetail::MissingPrincipal => 20,
        RuleMatchDetail::MissingScope => 10,
        RuleMatchDetail::Match | RuleMatchDetail::NoMatch => 0,
    }
}

pub fn remember_strongest_missing_requirement(
    current: &mut Option<(PolicyReasonCode, PolicyRuleId)>,
    reason: PolicyReasonCode,
    rule_id: PolicyRuleId,
) {
    let replace = current
        .as_ref()
        .map(|(current_reason, _)| {
            missing_requirement_rank(reason) > missing_requirement_rank(*current_reason)
        })
        .unwrap_or(true);
    if replace {
        *current = Some((reason, rule_id));
    }
}

fn missing_requirement_rank(reason: PolicyReasonCode) -> u8 {
    match reason {
        PolicyReasonCode::MissingDataLabel => 70,
        PolicyReasonCode::MissingPrincipalAssurance => 60,
        PolicyReasonCode::MissingRole => 50,
        PolicyReasonCode::MissingGroup => 40,
        PolicyReasonCode::MissingTenant => 30,
        PolicyReasonCode::MissingPrincipal => 20,
        PolicyReasonCode::MissingScope => 10,
        _ => 0,
    }
}

pub(crate) fn has_required_scopes(
    principal_scopes: &BTreeSet<veoveo_types::ScopeName>,
    required: &[ScopeName],
) -> bool {
    required
        .iter()
        .all(|scope| principal_scopes.contains(scope))
}

pub fn intersects<T: Ord>(left: &BTreeSet<T>, right: &BTreeSet<T>) -> bool {
    left.iter().any(|value| right.contains(value))
}

fn validate_runtime_action(
    catalog: &impl PolicyCatalogView,
    action: &PolicyAction,
    target: &PolicyTarget,
) -> Result<(), veoveo_types::ExtensionError> {
    match action {
        PolicyAction::Kernel(action) => {
            catalog
                .registry()
                .action_key::<GatewayAction>()?
                .action(*action)?;
        }
        PolicyAction::Registered(handle) => {
            catalog.registry().check_action(handle)?;
            let descriptor = catalog
                .registry()
                .descriptor(handle.name())
                .ok_or_else(|| {
                    veoveo_types::ExtensionError::new("owner action descriptor is unbound")
                })?;
            if !descriptor
                .target_kinds
                .iter()
                .any(|kind| kind.as_str() == target.kind())
            {
                return Err(veoveo_types::ExtensionError::new(
                    "action does not support this target",
                ));
            }
        }
    }
    match target {
        PolicyTarget::Owner(target) => catalog.registry().check_target(target),
        PolicyTarget::Unadmitted(_) => Err(veoveo_types::ExtensionError::new(
            "owner target is unadmitted",
        )),
        _ => Ok(()),
    }
}
