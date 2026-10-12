use veoveo_types::{LocalToolName, PermissionName, PromptName, ResourceTypeName};

/// The exact reason a declaration is rejected. Diagnostics name declared identifiers,
/// never request contents or secrets.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeclarationError {
    #[error("permission {permission} is outside the server's namespace")]
    PermissionOutsideNamespace { permission: PermissionName },
    #[error("resource type {resource} is outside the server's namespace")]
    ResourceOutsideNamespace { resource: ResourceTypeName },
    #[error("permission {permission} is declared more than once")]
    DuplicatePermission { permission: PermissionName },
    #[error("a requirement names undeclared permission {permission}")]
    UndeclaredPermission { permission: PermissionName },
    #[error("{target} is required twice with different permissions")]
    ConflictingRequirement { target: String },
    #[error("resource requirement {uri} is outside the server's URI scheme")]
    ResourceOutsideScheme { uri: String },
    #[error("resource template {template} must use simple {{variable}} expressions")]
    UnsupportedTemplate { template: String },
    #[error("tool {tool} is annotated read-only but requires a write permission")]
    ReadOnlyToolRequiresWrite { tool: LocalToolName },
    #[error("tool {tool} is annotated destructive but its permission lacks the destructive risk")]
    DestructiveToolWithoutRisk { tool: LocalToolName },
    #[error("prompt {prompt} requires a write permission")]
    PromptRequiresWrite { prompt: PromptName },
    #[error("invalid {field}: {reason}")]
    Invalid {
        field: &'static str,
        reason: &'static str,
    },
}

impl DeclarationError {
    pub(crate) fn invalid(field: &'static str, reason: &'static str) -> Self {
        Self::Invalid { field, reason }
    }
}
