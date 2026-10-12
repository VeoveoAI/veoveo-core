//! `ai.veoveo/authorization`: the permissions a server declares and the permission
//! each of its tools, resources, resource templates and prompts requires.
use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use veoveo_types::{
    LocalToolName, PermissionName, PromptName, ResourceScheme, ResourceTemplateUri,
    ResourceTypeName, ResourceUri, ResourceUriTemplate, ServerSlug, Vocabulary,
};

use crate::{Access, DeclarationError, Risk};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PermissionDeclaration {
    pub permission: PermissionName,
    pub resource: ResourceTypeName,
    pub access: Access,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub risks: BTreeSet<Risk>,
}

/// Each target names exactly one permission. A target the declaration omits is denied.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Requirements {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<LocalToolName, PermissionName>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resources: BTreeMap<ResourceUri, PermissionName>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resource_templates: BTreeMap<ResourceTemplateUri, PermissionName>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub prompts: BTreeMap<PromptName, PermissionName>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorizationDeclaration {
    pub permissions: Vec<PermissionDeclaration>,
    pub requirements: Requirements,
}

/// One request target whose required permission the Gateway looks up.
#[derive(Debug, Clone, Copy)]
pub enum RequirementTarget<'a> {
    Tool(&'a LocalToolName),
    Resource(&'a ResourceUri),
    ResourceTemplate(&'a ResourceTemplateUri),
    Prompt(&'a PromptName),
}

impl AuthorizationDeclaration {
    /// Check a declaration against the namespace pinned to the registering server.
    pub fn check(
        &self,
        server: &ServerSlug,
        uri_scheme: &ResourceScheme,
    ) -> Result<(), DeclarationError> {
        let mut declared = BTreeSet::new();
        for permission in &self.permissions {
            if !permission.permission.is_owned_by(server) {
                return Err(DeclarationError::PermissionOutsideNamespace {
                    permission: permission.permission.clone(),
                });
            }
            if !permission.resource.is_owned_by(server) {
                return Err(DeclarationError::ResourceOutsideNamespace {
                    resource: permission.resource.clone(),
                });
            }
            if !declared.insert(&permission.permission) {
                return Err(DeclarationError::DuplicatePermission {
                    permission: permission.permission.clone(),
                });
            }
        }
        let requirements = &self.requirements;
        for required in requirements
            .tools
            .values()
            .chain(requirements.resources.values())
            .chain(requirements.resource_templates.values())
            .chain(requirements.prompts.values())
        {
            if !declared.contains(required) {
                return Err(DeclarationError::UndeclaredPermission {
                    permission: required.clone(),
                });
            }
        }
        for uri in requirements.resources.keys() {
            if !owns_uri(server, uri_scheme, uri.as_str()) {
                return Err(DeclarationError::ResourceOutsideScheme {
                    uri: uri.as_str().to_owned(),
                });
            }
        }
        for template in requirements.resource_templates.keys() {
            if !owns_uri(server, uri_scheme, template.as_str()) {
                return Err(DeclarationError::ResourceOutsideScheme {
                    uri: template.as_str().to_owned(),
                });
            }
            if ResourceUriTemplate::new(template.as_str()).is_err() {
                return Err(DeclarationError::UnsupportedTemplate {
                    template: template.as_str().to_owned(),
                });
            }
        }
        for (prompt, permission) in &requirements.prompts {
            if self
                .permission(permission)
                .is_some_and(|declared| declared.access == Access::Write)
            {
                return Err(DeclarationError::PromptRequiresWrite {
                    prompt: prompt.clone(),
                });
            }
        }
        Ok(())
    }

    pub fn permission(&self, name: &PermissionName) -> Option<&PermissionDeclaration> {
        self.permissions
            .iter()
            .find(|permission| &permission.permission == name)
    }

    /// The permission a target requires. An exact resource entry wins over templates;
    /// a resource matched by templates that require different permissions has none.
    pub fn required(&self, target: RequirementTarget<'_>) -> Option<&PermissionDeclaration> {
        let requirements = &self.requirements;
        let name = match target {
            RequirementTarget::Tool(tool) => requirements.tools.get(tool),
            RequirementTarget::Prompt(prompt) => requirements.prompts.get(prompt),
            RequirementTarget::ResourceTemplate(template) => {
                requirements.resource_templates.get(template)
            }
            RequirementTarget::Resource(uri) => requirements.resources.get(uri).or_else(|| {
                let mut matched = requirements
                    .resource_templates
                    .iter()
                    .filter(|(template, _)| {
                        ResourceUriTemplate::new(template.as_str())
                            .is_ok_and(|template| template.matches_uri(uri))
                    })
                    .map(|(_, permission)| permission);
                let first = matched.next()?;
                matched.all(|other| other == first).then_some(first)
            }),
        }?;
        self.permission(name)
    }
}

/// A server's resources use its own scheme, or `ui://{slug}/` for its Apps.
fn owns_uri(server: &ServerSlug, scheme: &ResourceScheme, uri: &str) -> bool {
    uri.split_once("://").is_some_and(|(found, rest)| {
        found == scheme.as_str()
            || (found == "ui"
                && rest
                    .strip_prefix(server.as_str())
                    .is_some_and(|tail| tail.starts_with('/')))
    })
}

/// A server's closed permission vocabulary. One wire spelling per variant, such as
/// `time.events.write`; the builder checks each spelling once.
pub trait PermissionDefinition: Vocabulary + Ord {
    /// The resource type this permission acts on, such as `time.event`.
    fn resource(self) -> &'static str;
    fn access(self) -> Access;
    fn risks(self) -> &'static [Risk] {
        &[]
    }
}

/// Builds a declaration from a server's typed permissions. The same requirement table
/// that drives the declaration should drive the server's own handler checks.
pub struct AuthorizationBuilder<P: PermissionDefinition> {
    tools: BTreeMap<LocalToolName, P>,
    resources: BTreeMap<ResourceUri, P>,
    resource_templates: BTreeMap<ResourceTemplateUri, P>,
    prompts: BTreeMap<PromptName, P>,
    conflict: Option<String>,
}

impl<P: PermissionDefinition> Default for AuthorizationBuilder<P> {
    fn default() -> Self {
        Self {
            tools: BTreeMap::new(),
            resources: BTreeMap::new(),
            resource_templates: BTreeMap::new(),
            prompts: BTreeMap::new(),
            conflict: None,
        }
    }
}

fn require<K: Ord + Clone, P: Copy + Eq>(
    map: &mut BTreeMap<K, P>,
    key: K,
    permission: P,
    label: impl FnOnce(&K) -> String,
    conflict: &mut Option<String>,
) {
    if let Some(previous) = map.insert(key.clone(), permission)
        && previous != permission
        && conflict.is_none()
    {
        *conflict = Some(label(&key));
    }
}

impl<P: PermissionDefinition> AuthorizationBuilder<P> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tool(mut self, tool: LocalToolName, permission: P) -> Self {
        require(
            &mut self.tools,
            tool,
            permission,
            |tool| format!("tool {tool}"),
            &mut self.conflict,
        );
        self
    }

    pub fn resource(mut self, uri: ResourceUri, permission: P) -> Self {
        require(
            &mut self.resources,
            uri,
            permission,
            |uri| format!("resource {}", uri.as_str()),
            &mut self.conflict,
        );
        self
    }

    pub fn resource_template(mut self, template: ResourceTemplateUri, permission: P) -> Self {
        require(
            &mut self.resource_templates,
            template,
            permission,
            |template| format!("resource template {}", template.as_str()),
            &mut self.conflict,
        );
        self
    }

    pub fn prompt(mut self, prompt: PromptName, permission: P) -> Self {
        require(
            &mut self.prompts,
            prompt,
            permission,
            |prompt| format!("prompt {prompt}"),
            &mut self.conflict,
        );
        self
    }

    /// Declare every permission in `P` and check the result against the server's
    /// namespace, exactly as the Gateway will.
    pub fn build(
        self,
        server: &ServerSlug,
        uri_scheme: &ResourceScheme,
    ) -> Result<AuthorizationDeclaration, DeclarationError> {
        if let Some(target) = self.conflict {
            return Err(DeclarationError::ConflictingRequirement { target });
        }
        let name = permission_name::<P>;
        let permissions = P::ALL
            .iter()
            .map(|permission| {
                Ok(PermissionDeclaration {
                    permission: name(*permission)?,
                    resource: ResourceTypeName::parse(permission.resource()).map_err(|_| {
                        DeclarationError::invalid("resource", "invalid resource type spelling")
                    })?,
                    access: permission.access(),
                    risks: permission.risks().iter().copied().collect(),
                })
            })
            .collect::<Result<Vec<_>, DeclarationError>>()?;
        let declaration = AuthorizationDeclaration {
            permissions,
            requirements: Requirements {
                tools: permission_names(self.tools)?,
                resources: permission_names(self.resources)?,
                resource_templates: permission_names(self.resource_templates)?,
                prompts: permission_names(self.prompts)?,
            },
        };
        declaration.check(server, uri_scheme)?;
        Ok(declaration)
    }
}

fn permission_name<P: PermissionDefinition>(
    permission: P,
) -> Result<PermissionName, DeclarationError> {
    PermissionName::parse(permission.as_str())
        .map_err(|_| DeclarationError::invalid("permission", "invalid permission spelling"))
}

fn permission_names<K: Ord, P: PermissionDefinition>(
    map: BTreeMap<K, P>,
) -> Result<BTreeMap<K, PermissionName>, DeclarationError> {
    map.into_iter()
        .map(|(key, permission)| Ok((key, permission_name(permission)?)))
        .collect()
}
