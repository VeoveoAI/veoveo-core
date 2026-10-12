//! `ai.veoveo/module`: the tables, functions and migrations a server owns. The Gateway
//! admits the manifest through the same checks as a compiled module declaration and
//! applies its migrations with its own database credentials.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use veoveo_modules::{ManifestClaim, MigrationVersion, ModuleManifest, ModuleName};
use veoveo_types::ServerSlug;

use crate::DeclarationError;

/// The full manifest, sent as registration metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ModuleDeclaration(ModuleManifest);

/// Advertised in MCP discovery instead of the full manifest, so discovery responses
/// stay small. Equal digests identify equal manifests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModuleSummary {
    pub name: ModuleName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<MigrationVersion>,
    pub sha256: veoveo_types::Sha256Digest,
}

impl ModuleDeclaration {
    pub fn new(module: ModuleManifest) -> Self {
        Self(module)
    }

    pub fn manifest(&self) -> &ModuleManifest {
        &self.0
    }

    pub fn into_manifest(self) -> ModuleManifest {
        self.0
    }

    /// A registering server's module is named after its slug and claims only names
    /// that start with `{slug}_` (tables and analyzers) or `fn::{slug}::` (functions),
    /// with hyphens in the slug written as underscores.
    pub fn check(&self, server: &ServerSlug) -> Result<(), DeclarationError> {
        if self.0.name.as_str() != server.as_str() {
            return Err(DeclarationError::invalid(
                "module",
                "module name must equal the server slug",
            ));
        }
        let slug = server.as_str().replace('-', "_");
        let table_prefix = format!("{slug}_");
        let function_prefix = format!("fn::{slug}::");
        for claim in &self.0.ownership {
            let owned = match claim {
                ManifestClaim::Table(name) => name.as_str().starts_with(&table_prefix),
                ManifestClaim::TablePrefix(name) => name.as_str().starts_with(&table_prefix),
                ManifestClaim::Function(name) => name.as_str().starts_with(&function_prefix),
                ManifestClaim::FunctionPrefix(name) => name.as_str().starts_with(&function_prefix),
                ManifestClaim::Analyzer(name) => name.as_str().starts_with(&table_prefix),
            };
            if !owned {
                return Err(DeclarationError::invalid(
                    "module",
                    "ownership claims must stay within the server's table and function prefixes",
                ));
            }
        }
        Ok(())
    }

    pub fn summary(&self) -> ModuleSummary {
        ModuleSummary {
            name: self.0.name.clone(),
            latest: self.0.latest(),
            sha256: veoveo_types::Sha256Digest::from_hex(self.0.sha256())
                .expect("manifest digest is SHA-256 hexadecimal"),
        }
    }
}
