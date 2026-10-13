//! The installation's audit registry: the class declarations that appends, queries and
//! readers admit records with. A registry is immutable once built.
use crate::{AuditClass, AuditDetailDeclaration};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
pub struct AuditRegistry {
    pub(crate) details: Arc<BTreeMap<AuditClass, AuditDetailDeclaration>>,
}

impl std::fmt::Debug for AuditRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditRegistry")
            .field("classes", &self.details.keys())
            .finish()
    }
}

pub struct AuditRegistryBuilder {
    pub(crate) details: BTreeMap<AuditClass, AuditDetailDeclaration>,
}

/// A reader schema could not be composed from the registry's declarations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuditSchemaError {
    #[error("the reader schema has no definitions to compose")]
    Invalid,
    #[error("audit detail schema definition collision `{0}`")]
    Collision(String),
}

impl AuditRegistry {
    /// A builder that already holds the platform's declarations.
    pub fn builder() -> AuditRegistryBuilder {
        let mut builder = AuditRegistryBuilder {
            details: BTreeMap::new(),
        };
        builder.declare_platform();
        builder
    }

    /// A registry with only the platform's classes.
    pub fn platform() -> Self {
        Self::builder().build()
    }

    /// Compose a schema root whose `AuditClass` and `AuditDetailFields` definitions admit
    /// exactly this registry's classes and their documented detail schemas.
    pub(crate) fn compose_schema(
        &self,
        mut root: schemars::Schema,
    ) -> Result<schemars::Schema, AuditSchemaError> {
        let object = root.as_object_mut().ok_or(AuditSchemaError::Invalid)?;
        let defs = object
            .get_mut("$defs")
            .and_then(Value::as_object_mut)
            .ok_or(AuditSchemaError::Invalid)?;
        let mut details = Vec::new();
        let mut open = false;
        for declaration in self.details.values() {
            match declaration.schema() {
                Some(schema) => details.push(merge_definitions(defs, schema.clone())?),
                None => open = true,
            }
        }
        let classes: Vec<Value> = self
            .details
            .keys()
            .map(|class| Value::String(class.as_str().to_owned()))
            .collect();
        if let Some(class) = defs.get_mut("AuditClass").and_then(Value::as_object_mut) {
            class.insert("enum".into(), Value::Array(classes));
        }
        if let Some(fields) = defs.get_mut("AuditDetailFields") {
            if open {
                details.push(serde_json::json!({"type": "object"}));
            }
            *fields = serde_json::json!({ "anyOf": details });
        }
        Ok(root)
    }
}

impl AuditRegistryBuilder {
    pub fn build(self) -> AuditRegistry {
        AuditRegistry {
            details: Arc::new(self.details),
        }
    }
}

/// Move an owner schema's local definitions into the root and return the schema itself.
fn merge_definitions(
    defs: &mut serde_json::Map<String, Value>,
    mut schema: Value,
) -> Result<Value, AuditSchemaError> {
    let object = schema.as_object_mut().ok_or(AuditSchemaError::Invalid)?;
    object.remove("$schema");
    if let Some(Value::Object(owner)) = object.remove("$defs") {
        for (name, definition) in owner {
            match defs.get(&name) {
                Some(existing) if existing == &definition => {}
                Some(_) => return Err(AuditSchemaError::Collision(name)),
                None => {
                    defs.insert(name, definition);
                }
            }
        }
    }
    Ok(schema)
}
