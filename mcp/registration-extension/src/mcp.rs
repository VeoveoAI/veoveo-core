//! MCP discovery: a server advertises its declarations in `capabilities.extensions`,
//! so what it registers and what it advertises have one source. The Gateway reads them
//! back and checks them against the tools the server lists.
use rmcp::model::{JsonObject, ServerCapabilities, Tool};
use veoveo_types::LocalToolName;

use crate::{
    AUTHORIZATION_EXTENSION_ID, Access, AuthorizationDeclaration, DeclarationError,
    MODULE_EXTENSION_ID, ModuleSummary, REGISTRATION_EXTENSION_ID, RegistrationDeclaration, Risk,
};

/// Advertise the declarations. A server with a module advertises its summary.
pub fn advertise(
    capabilities: &mut ServerCapabilities,
    registration: &RegistrationDeclaration,
    authorization: &AuthorizationDeclaration,
    module: Option<&ModuleSummary>,
) {
    let extensions = capabilities.extensions.get_or_insert_default();
    extensions.insert(REGISTRATION_EXTENSION_ID.into(), object(registration));
    extensions.insert(AUTHORIZATION_EXTENSION_ID.into(), object(authorization));
    if let Some(module) = module {
        extensions.insert(MODULE_EXTENSION_ID.into(), object(module));
    }
}

fn object(value: &impl serde::Serialize) -> JsonObject {
    match serde_json::to_value(value).expect("typed declaration serializes") {
        serde_json::Value::Object(object) => object,
        _ => unreachable!("declarations serialize as JSON objects"),
    }
}

/// What a server advertises in its discovery result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advertised {
    pub registration: RegistrationDeclaration,
    pub authorization: AuthorizationDeclaration,
    pub module: Option<ModuleSummary>,
}

pub fn advertised(
    capabilities: &ServerCapabilities,
) -> Result<Option<Advertised>, DeclarationError> {
    let Some(extensions) = capabilities.extensions.as_ref() else {
        return Ok(None);
    };
    let registration: Option<RegistrationDeclaration> =
        read(extensions, REGISTRATION_EXTENSION_ID, "registration")?;
    let authorization: Option<AuthorizationDeclaration> =
        read(extensions, AUTHORIZATION_EXTENSION_ID, "authorization")?;
    let module: Option<ModuleSummary> = read(extensions, MODULE_EXTENSION_ID, "module")?;
    match (registration, authorization) {
        (None, None) if module.is_none() => Ok(None),
        (Some(registration), Some(authorization)) => Ok(Some(Advertised {
            registration,
            authorization,
            module,
        })),
        _ => Err(DeclarationError::invalid(
            "capabilities",
            "registration and authorization must be advertised together",
        )),
    }
}

fn read<T: serde::de::DeserializeOwned>(
    extensions: &rmcp::model::ExtensionCapabilities,
    id: &str,
    field: &'static str,
) -> Result<Option<T>, DeclarationError> {
    extensions
        .get(id)
        .map(|object| {
            serde_json::from_value(serde_json::Value::Object(object.clone()))
                .map_err(|_| DeclarationError::invalid(field, "does not match its declared shape"))
        })
        .transpose()
}

/// MCP annotations are hints that a client must not trust for security; here they
/// only catch a declaration that contradicts the server's own tool description. A
/// read-only tool must require read access, and a destructive tool must carry the
/// destructive risk. Absent hints impose nothing.
pub fn check_tool_annotations(
    declaration: &AuthorizationDeclaration,
    tools: &[Tool],
) -> Result<(), DeclarationError> {
    for tool in tools {
        let Some(annotations) = tool.annotations.as_ref() else {
            continue;
        };
        let Ok(name) = LocalToolName::parse(tool.name.as_ref()) else {
            continue;
        };
        let Some(permission) = declaration
            .requirements
            .tools
            .get(&name)
            .and_then(|required| declaration.permission(required))
        else {
            continue;
        };
        if annotations.read_only_hint == Some(true) && permission.access == Access::Write {
            return Err(DeclarationError::ReadOnlyToolRequiresWrite { tool: name });
        }
        if annotations.destructive_hint == Some(true)
            && !permission.risks.contains(&Risk::Destructive)
        {
            return Err(DeclarationError::DestructiveToolWithoutRisk { tool: name });
        }
    }
    Ok(())
}
