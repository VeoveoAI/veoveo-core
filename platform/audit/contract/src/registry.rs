//! The installation's audit registry: the owner target codecs and the detail
//! declarations that appends, queries and readers admit records with. A registry is
//! immutable once built, and clones share its identity.
use crate::{AuditClass, AuditTargetError, detail::DetailCodec, targets::Codec};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
pub struct AuditRegistry {
    pub(crate) identity: Arc<()>,
    pub(crate) codecs: Arc<BTreeMap<String, Codec>>,
    pub(crate) details: Arc<BTreeMap<AuditClass, DetailCodec>>,
}

impl std::fmt::Debug for AuditRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditRegistry")
            .field("targets", &self.codecs.keys())
            .field("classes", &self.details.keys())
            .finish()
    }
}

pub struct AuditRegistryBuilder {
    pub(crate) identity: Arc<()>,
    pub(crate) codecs: BTreeMap<String, Codec>,
    pub(crate) details: BTreeMap<AuditClass, DetailCodec>,
}

impl AuditRegistry {
    /// A builder that already holds the platform's detail declarations.
    pub fn builder() -> AuditRegistryBuilder {
        let mut builder = AuditRegistryBuilder {
            identity: Arc::new(()),
            codecs: BTreeMap::new(),
            details: BTreeMap::new(),
        };
        builder.declare_platform();
        builder
    }

    /// A registry with only the platform's targets and classes.
    pub fn platform() -> Self {
        Self::builder().build()
    }

    /// Compose a schema root whose `AuditTarget`, `AuditClass` and `AuditDetailFields`
    /// definitions admit exactly this registry's targets and declarations.
    pub(crate) fn compose_schema(
        &self,
        mut root: schemars::Schema,
    ) -> Result<schemars::Schema, AuditTargetError> {
        let object = root.as_object_mut().ok_or(AuditTargetError::Invalid)?;
        let defs = object
            .get_mut("$defs")
            .and_then(Value::as_object_mut)
            .ok_or(AuditTargetError::Invalid)?;
        let mut targets = Vec::new();
        for codec in self.codecs.values() {
            targets.push(merge_definitions(defs, codec.schema.clone())?);
        }
        let mut details = Vec::new();
        for codec in self.details.values() {
            details.push(merge_definitions(defs, codec.declaration.schema().clone())?);
        }
        defs.get_mut("AuditTarget")
            .and_then(|target| target.get_mut("oneOf"))
            .and_then(Value::as_array_mut)
            .ok_or(AuditTargetError::Invalid)?
            .extend(targets);
        let classes: Vec<Value> = self
            .details
            .keys()
            .map(|class| Value::String(class.as_str().to_owned()))
            .collect();
        if let Some(class) = defs.get_mut("AuditClass").and_then(Value::as_object_mut) {
            class.insert("enum".into(), Value::Array(classes));
        }
        if let Some(fields) = defs.get_mut("AuditDetailFields") {
            *fields = serde_json::json!({ "anyOf": details });
        }
        Ok(root)
    }
}

impl AuditRegistryBuilder {
    pub fn build(self) -> AuditRegistry {
        AuditRegistry {
            identity: self.identity,
            codecs: Arc::new(self.codecs),
            details: Arc::new(self.details),
        }
    }
}

/// Move an owner schema's local definitions into the root and return the schema itself.
fn merge_definitions(
    defs: &mut serde_json::Map<String, Value>,
    mut schema: Value,
) -> Result<Value, AuditTargetError> {
    let object = schema.as_object_mut().ok_or(AuditTargetError::Invalid)?;
    object.remove("$schema");
    if let Some(Value::Object(owner)) = object.remove("$defs") {
        for (name, definition) in owner {
            match defs.get(&name) {
                Some(existing) if existing == &definition => {}
                Some(_) => return Err(AuditTargetError::SchemaCollision(name)),
                None => {
                    defs.insert(name, definition);
                }
            }
        }
    }
    Ok(schema)
}
