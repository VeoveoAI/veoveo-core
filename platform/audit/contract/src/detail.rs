//! Owner-declared audit details. Each owner declares a class together with the closed
//! JSON Schema of its detail, and a registry admits a detail only against its class's
//! declaration. Writers check every detail against that schema; readers require only
//! that the class is declared, so a record keeps reading after its owner's schema
//! changes.
use crate::{AuditClass, AuditRegistry, AuditRegistryBuilder, targets::check_closed_schema};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuditDetailError {
    #[error(
        "audit class `{0}` is not declared; declare the owner's detail before recording or reading it"
    )]
    Undeclared(AuditClass),
    #[error("audit class `{0}` is declared twice with different schemas")]
    Collision(AuditClass),
    #[error("audit class `{0}` belongs to a platform owner")]
    Reserved(AuditClass),
    #[error("invalid audit class `{0}`")]
    Class(String),
    #[error("the detail schema of audit class `{0}` is not a closed object schema")]
    Schema(AuditClass),
    #[error("audit detail does not match the declared schema of `{0}`")]
    Invalid(AuditClass),
    #[error("an audit detail of class `{actual}` was read as `{expected}`")]
    Mismatch {
        expected: AuditClass,
        actual: AuditClass,
    },
}

/// A detail type an owner records under one class. Its schema must be a closed object.
pub trait AuditDetailType: Serialize + DeserializeOwned + JsonSchema {
    /// The class, `<owner>.<name>`.
    const CLASS: &'static str;
}

/// The fields of a recorded detail, a JSON object its class's schema admitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct AuditDetailFields(Map<String, Value>);

impl AuditDetailFields {
    pub fn as_map(&self) -> &Map<String, Value> {
        &self.0
    }
}

impl JsonSchema for AuditDetailFields {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "AuditDetailFields".into()
    }
    /// Without declarations a detail is the empty object; a reader schema replaces this
    /// definition with the declared detail schemas.
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"type": "object", "additionalProperties": false})
    }
}

/// A detail admitted against its class's declaration, ready for a draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditDetail {
    class: AuditClass,
    fields: AuditDetailFields,
}

impl AuditDetail {
    pub fn class(&self) -> &AuditClass {
        &self.class
    }

    pub fn fields(&self) -> &AuditDetailFields {
        &self.fields
    }

    pub(crate) fn into_parts(self) -> (AuditClass, AuditDetailFields) {
        (self.class, self.fields)
    }
}

/// One class's declaration: the class and the closed JSON Schema of its detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditDetailDeclaration {
    class: AuditClass,
    schema: Value,
}

impl AuditDetailDeclaration {
    /// The declaration of an owner's detail type, with the schema its type generates.
    pub fn of<T: AuditDetailType>() -> Result<Self, AuditDetailError> {
        let class =
            AuditClass::parse(T::CLASS).map_err(|_| AuditDetailError::Class(T::CLASS.into()))?;
        let schema = serde_json::to_value(schemars::schema_for!(T))
            .map_err(|_| AuditDetailError::Schema(class.clone()))?;
        Self::new(class, schema)
    }

    /// A declaration read back from storage or received from its owner. The schema must
    /// describe a closed object in the admitted schema profile.
    pub fn new(class: AuditClass, schema: Value) -> Result<Self, AuditDetailError> {
        let closed_object = schema.get("type") == Some(&Value::String("object".into()))
            && schema.get("additionalProperties") == Some(&Value::Bool(false));
        if !closed_object || check_closed_schema(&schema, &schema).is_err() {
            return Err(AuditDetailError::Schema(class));
        }
        Ok(Self { class, schema })
    }

    pub fn class(&self) -> &AuditClass {
        &self.class
    }

    pub fn schema(&self) -> &Value {
        &self.schema
    }
}

/// A declaration with its compiled validator.
#[derive(Clone)]
pub(crate) struct DetailCodec {
    pub(crate) declaration: AuditDetailDeclaration,
    validator: Arc<jsonschema::Validator>,
}

impl DetailCodec {
    fn new(declaration: AuditDetailDeclaration) -> Result<Self, AuditDetailError> {
        let validator = jsonschema::validator_for(&declaration.schema)
            .map_err(|_| AuditDetailError::Schema(declaration.class.clone()))?;
        Ok(Self {
            declaration,
            validator: Arc::new(validator),
        })
    }
}

impl AuditRegistryBuilder {
    /// Declare an owner's detail type under its class.
    pub fn declare<T: AuditDetailType>(&mut self) -> Result<(), AuditDetailError> {
        self.declaration(AuditDetailDeclaration::of::<T>()?)
    }

    /// Add a declaration, such as one another owner stored. Repeating an identical
    /// declaration changes nothing; a different schema for a declared class, or a class
    /// of a platform owner that the platform did not declare, is refused.
    pub fn declaration(
        &mut self,
        declaration: AuditDetailDeclaration,
    ) -> Result<(), AuditDetailError> {
        if let Some(existing) = self.details.get(&declaration.class) {
            return if existing.declaration == declaration {
                Ok(())
            } else {
                Err(AuditDetailError::Collision(declaration.class))
            };
        }
        if platform::OWNERS.contains(&declaration.class.owner()) {
            return Err(AuditDetailError::Reserved(declaration.class));
        }
        self.details
            .insert(declaration.class.clone(), DetailCodec::new(declaration)?);
        Ok(())
    }

    pub(crate) fn declare_platform(&mut self) {
        for declaration in platform::declarations() {
            let codec = DetailCodec::new(declaration).expect("platform details compile");
            self.details.insert(codec.declaration.class.clone(), codec);
        }
    }
}

impl AuditRegistry {
    /// Admit an owner's detail against its class's declaration.
    pub fn detail<T: AuditDetailType>(&self, value: &T) -> Result<AuditDetail, AuditDetailError> {
        let class =
            AuditClass::parse(T::CLASS).map_err(|_| AuditDetailError::Class(T::CLASS.into()))?;
        let Value::Object(fields) =
            serde_json::to_value(value).map_err(|_| AuditDetailError::Invalid(class.clone()))?
        else {
            return Err(AuditDetailError::Invalid(class));
        };
        let fields = AuditDetailFields(fields);
        self.check_detail(&class, &fields)?;
        Ok(AuditDetail { class, fields })
    }

    /// Check a detail against its class's declared schema, as every writer does.
    pub fn check_detail(
        &self,
        class: &AuditClass,
        fields: &AuditDetailFields,
    ) -> Result<(), AuditDetailError> {
        let codec = self.codec(class)?;
        let value = Value::Object(fields.0.clone());
        if codec.validator.is_valid(&value) {
            Ok(())
        } else {
            Err(AuditDetailError::Invalid(class.clone()))
        }
    }

    /// Read a recorded detail as its owner's type.
    pub fn read<T: AuditDetailType>(
        &self,
        class: &AuditClass,
        fields: &AuditDetailFields,
    ) -> Result<T, AuditDetailError> {
        let expected =
            AuditClass::parse(T::CLASS).map_err(|_| AuditDetailError::Class(T::CLASS.into()))?;
        if &expected != class {
            return Err(AuditDetailError::Mismatch {
                expected,
                actual: class.clone(),
            });
        }
        self.codec(class)?;
        serde_json::from_value(Value::Object(fields.0.clone()))
            .map_err(|_| AuditDetailError::Invalid(class.clone()))
    }

    /// Every declaration this registry holds, in class order.
    pub fn declarations(&self) -> impl Iterator<Item = &AuditDetailDeclaration> {
        self.details.values().map(|codec| &codec.declaration)
    }

    /// Admit a stored detail of a declared class. Reading checks only the declaration,
    /// so records keep reading after their owner's schema changes.
    pub(crate) fn admit_detail(
        &self,
        class: &AuditClass,
        fields: Map<String, Value>,
    ) -> Result<AuditDetailFields, AuditDetailError> {
        self.codec(class)?;
        Ok(AuditDetailFields(fields))
    }

    fn codec(&self, class: &AuditClass) -> Result<&DetailCodec, AuditDetailError> {
        self.details
            .get(class)
            .ok_or_else(|| AuditDetailError::Undeclared(class.clone()))
    }
}

/// The classes the platform declares. Every registry holds them, and no other owner may
/// declare a class under their owners.
pub mod platform {
    use super::*;

    /// The owners of platform classes.
    pub(crate) const OWNERS: [&str; 2] = ["task", "audit"];

    /// A caller changed a Task's input, status or retention: class `task.update`.
    #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct TaskUpdate {}
    impl AuditDetailType for TaskUpdate {
        const CLASS: &'static str = "task.update";
    }

    /// A caller cancelled a Task: class `task.cancel`.
    #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct TaskCancel {}
    impl AuditDetailType for TaskCancel {
        const CLASS: &'static str = "task.cancel";
    }

    /// A caller opened a view of an audit partition: class `audit.view`. The record is
    /// the receipt that later reads of the view present.
    #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct AuditView {}
    impl AuditDetailType for AuditView {
        const CLASS: &'static str = "audit.view";
    }

    /// A caller exported records of an audit partition: class `audit.export`.
    #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct AuditExport {}
    impl AuditDetailType for AuditExport {
        const CLASS: &'static str = "audit.export";
    }

    pub(crate) fn declarations() -> [AuditDetailDeclaration; 4] {
        [
            AuditDetailDeclaration::of::<TaskUpdate>(),
            AuditDetailDeclaration::of::<TaskCancel>(),
            AuditDetailDeclaration::of::<AuditView>(),
            AuditDetailDeclaration::of::<AuditExport>(),
        ]
        .map(|declaration| declaration.expect("platform detail declarations are closed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct Note {
        count: u32,
    }
    impl AuditDetailType for Note {
        const CLASS: &'static str = "fixture.note";
    }

    #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
    struct Open {
        count: u32,
    }
    impl AuditDetailType for Open {
        const CLASS: &'static str = "fixture.open";
    }

    #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct Platform {}
    impl AuditDetailType for Platform {
        const CLASS: &'static str = "task.forge";
    }

    fn registry() -> AuditRegistry {
        let mut builder = AuditRegistry::builder();
        builder.declare::<Note>().unwrap();
        builder.build()
    }

    #[test]
    fn details_are_checked_against_their_declared_schema() {
        let registry = registry();
        let detail = registry.detail(&Note { count: 3 }).unwrap();
        assert_eq!(detail.class().as_str(), "fixture.note");
        assert_eq!(
            registry
                .read::<Note>(detail.class(), detail.fields())
                .unwrap(),
            Note { count: 3 }
        );
        let mut extra = detail.fields().as_map().clone();
        extra.insert("secret".into(), "text".into());
        assert_eq!(
            registry.check_detail(detail.class(), &AuditDetailFields(extra)),
            Err(AuditDetailError::Invalid(detail.class().clone()))
        );
        let undeclared = AuditClass::parse("fixture.other").unwrap();
        assert_eq!(
            registry.check_detail(&undeclared, detail.fields()),
            Err(AuditDetailError::Undeclared(undeclared))
        );
        assert!(matches!(
            registry.read::<platform::TaskUpdate>(detail.class(), detail.fields()),
            Err(AuditDetailError::Mismatch { .. })
        ));
    }

    #[test]
    fn declarations_are_closed_unique_and_outside_platform_owners() {
        let mut builder = AuditRegistry::builder();
        assert!(matches!(
            builder.declare::<Open>(),
            Err(AuditDetailError::Schema(_))
        ));
        assert!(matches!(
            builder.declare::<Platform>(),
            Err(AuditDetailError::Reserved(_))
        ));
        builder.declare::<Note>().unwrap();
        builder.declare::<Note>().unwrap();
        let class = AuditClass::parse(Note::CLASS).unwrap();
        let other = AuditDetailDeclaration::new(
            class.clone(),
            serde_json::json!({"type":"object","additionalProperties":false}),
        )
        .unwrap();
        assert_eq!(
            builder.declaration(other),
            Err(AuditDetailError::Collision(class))
        );
        let platform = AuditDetailDeclaration::of::<platform::TaskCancel>().unwrap();
        builder.declaration(platform).unwrap();
    }

    #[test]
    fn platform_classes_belong_to_platform_owners_and_name_no_server() {
        let registry = AuditRegistry::platform();
        let classes: Vec<_> = registry
            .declarations()
            .map(|declaration| declaration.class().as_str().to_owned())
            .collect();
        assert_eq!(
            classes,
            ["audit.export", "audit.view", "task.cancel", "task.update"]
        );
        for declaration in registry.declarations() {
            assert!(platform::OWNERS.contains(&declaration.class().owner()));
        }
    }
}
