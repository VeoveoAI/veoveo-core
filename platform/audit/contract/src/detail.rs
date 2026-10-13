//! Owner-declared audit classes. Each owner declares a class with the OCSF class and
//! activity it maps to and, as documentation, the JSON Schema of its detail. A registry
//! admits a detail of a declared class whose JSON is an object of at most 16 KiB; the
//! owner's types keep the detail's shape, so the audit writer checks no schema.
use crate::{AuditClass, AuditRegistry, AuditRegistryBuilder};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};

/// The most JSON an audit detail holds, in bytes.
pub const MAX_DETAIL_BYTES: usize = 16 * 1024;

pub(crate) fn detail_size(fields: &Map<String, Value>) -> usize {
    serde_json::to_vec(fields).map_or(usize::MAX, |bytes| bytes.len())
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuditDetailError {
    #[error(
        "audit class `{0}` is not declared; declare the owner's detail before recording or reading it"
    )]
    Undeclared(AuditClass),
    #[error("audit class `{0}` is declared twice with different declarations")]
    Collision(AuditClass),
    #[error("audit class `{0}` belongs to a platform owner")]
    Reserved(AuditClass),
    #[error("invalid audit class `{0}`")]
    Class(String),
    #[error("the detail schema of audit class `{0}` is not a JSON Schema object")]
    Schema(AuditClass),
    #[error("an audit detail of `{0}` is not a JSON object of at most 16 KiB")]
    Invalid(AuditClass),
    #[error("an audit detail of class `{actual}` was read as `{expected}`")]
    Mismatch {
        expected: AuditClass,
        actual: AuditClass,
    },
}

/// The OCSF 1.9 event class and activity a Veoveo audit class maps to, such as API
/// Activity (6003) with Read (2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditOcsfClass {
    pub class_uid: u32,
    pub activity_id: u8,
}

impl AuditOcsfClass {
    /// OCSF API Activity (6003), whose activities are Create (1), Read (2), Update (3),
    /// Delete (4) and Other (99).
    pub const fn api_activity(activity_id: u8) -> Self {
        Self {
            class_uid: 6003,
            activity_id,
        }
    }

    /// An API Activity the owner names no OCSF activity for.
    pub const API_ACTIVITY_OTHER: Self = Self::api_activity(99);
}

/// A detail type an owner records under one class.
pub trait AuditDetailType: Serialize + DeserializeOwned + JsonSchema {
    /// The class, `<owner>.<name>`.
    const CLASS: &'static str;
    /// The OCSF class and activity the export maps the class to.
    const OCSF: AuditOcsfClass = AuditOcsfClass::API_ACTIVITY_OTHER;
}

/// The fields of a recorded detail: a JSON object of at most 16 KiB.
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
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"type": "object"})
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

/// One class's declaration: the class, the OCSF class and activity it maps to, and the
/// JSON Schema of its detail as documentation for readers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditDetailDeclaration {
    class: AuditClass,
    ocsf: AuditOcsfClass,
    schema: Option<Value>,
}

impl AuditDetailDeclaration {
    /// The declaration of an owner's detail type, with the schema its type generates.
    pub fn of<T: AuditDetailType>() -> Result<Self, AuditDetailError> {
        let class =
            AuditClass::parse(T::CLASS).map_err(|_| AuditDetailError::Class(T::CLASS.into()))?;
        let schema = serde_json::to_value(schemars::schema_for!(T))
            .map_err(|_| AuditDetailError::Schema(class.clone()))?;
        Self::new(class, T::OCSF, Some(schema))
    }

    /// A declaration read back from storage or received from its owner. A schema, when
    /// present, must be a JSON object.
    pub fn new(
        class: AuditClass,
        ocsf: AuditOcsfClass,
        schema: Option<Value>,
    ) -> Result<Self, AuditDetailError> {
        if schema.as_ref().is_some_and(|schema| !schema.is_object()) {
            return Err(AuditDetailError::Schema(class));
        }
        Ok(Self {
            class,
            ocsf,
            schema,
        })
    }

    pub fn class(&self) -> &AuditClass {
        &self.class
    }

    pub fn ocsf(&self) -> AuditOcsfClass {
        self.ocsf
    }

    pub fn schema(&self) -> Option<&Value> {
        self.schema.as_ref()
    }
}

impl AuditRegistryBuilder {
    /// Declare an owner's detail type under its class.
    pub fn declare<T: AuditDetailType>(&mut self) -> Result<(), AuditDetailError> {
        self.declaration(AuditDetailDeclaration::of::<T>()?)
    }

    /// Add a declaration, such as one another owner stored. Repeating an identical
    /// declaration changes nothing; a different declaration of a declared class, or a
    /// class of a platform owner that the platform did not declare, is refused.
    pub fn declaration(
        &mut self,
        declaration: AuditDetailDeclaration,
    ) -> Result<(), AuditDetailError> {
        if let Some(existing) = self.details.get(&declaration.class) {
            return if existing == &declaration {
                Ok(())
            } else {
                Err(AuditDetailError::Collision(declaration.class))
            };
        }
        if platform::OWNERS.contains(&declaration.class.owner()) {
            return Err(AuditDetailError::Reserved(declaration.class));
        }
        self.details.insert(declaration.class.clone(), declaration);
        Ok(())
    }

    pub(crate) fn declare_platform(&mut self) {
        for declaration in platform::declarations() {
            self.details.insert(declaration.class.clone(), declaration);
        }
    }
}

impl AuditRegistry {
    /// Admit an owner's detail of a declared class.
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

    /// Check that a detail's class is declared and its JSON fits the size limit, as
    /// every writer does.
    pub fn check_detail(
        &self,
        class: &AuditClass,
        fields: &AuditDetailFields,
    ) -> Result<(), AuditDetailError> {
        self.declaration(class)?;
        if detail_size(&fields.0) > MAX_DETAIL_BYTES {
            return Err(AuditDetailError::Invalid(class.clone()));
        }
        Ok(())
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
        self.declaration(class)?;
        serde_json::from_value(Value::Object(fields.0.clone()))
            .map_err(|_| AuditDetailError::Invalid(class.clone()))
    }

    /// Every declaration this registry holds, in class order.
    pub fn declarations(&self) -> impl Iterator<Item = &AuditDetailDeclaration> {
        self.details.values()
    }

    /// Admit a stored detail of a declared class. Reading checks only the declaration,
    /// so records keep reading after their owner's types change.
    pub(crate) fn admit_detail(
        &self,
        class: &AuditClass,
        fields: Map<String, Value>,
    ) -> Result<AuditDetailFields, AuditDetailError> {
        self.declaration(class)?;
        Ok(AuditDetailFields(fields))
    }

    /// The declaration of `class`.
    pub fn declaration(
        &self,
        class: &AuditClass,
    ) -> Result<&AuditDetailDeclaration, AuditDetailError> {
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
        const OCSF: AuditOcsfClass = AuditOcsfClass::api_activity(3);
    }

    /// A caller cancelled a Task: class `task.cancel`.
    #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct TaskCancel {}
    impl AuditDetailType for TaskCancel {
        const CLASS: &'static str = "task.cancel";
        const OCSF: AuditOcsfClass = AuditOcsfClass::api_activity(3);
    }

    /// A caller opened a view of an audit partition: class `audit.view`. The record is
    /// the receipt that later reads of the view present.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct AuditView {
        /// The partition the view reads.
        pub partition: crate::AuditPartition,
    }
    impl AuditDetailType for AuditView {
        const CLASS: &'static str = "audit.view";
        const OCSF: AuditOcsfClass = AuditOcsfClass::api_activity(2);
    }

    /// A caller exported records of an audit partition: class `audit.export`.
    #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct AuditExport {}
    impl AuditDetailType for AuditExport {
        const CLASS: &'static str = "audit.export";
        const OCSF: AuditOcsfClass = AuditOcsfClass::api_activity(2);
    }

    pub(crate) fn declarations() -> [AuditDetailDeclaration; 4] {
        [
            AuditDetailDeclaration::of::<TaskUpdate>(),
            AuditDetailDeclaration::of::<TaskCancel>(),
            AuditDetailDeclaration::of::<AuditView>(),
            AuditDetailDeclaration::of::<AuditExport>(),
        ]
        .map(|declaration| declaration.expect("platform detail declarations are objects"))
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
    struct Big {
        text: String,
    }
    impl AuditDetailType for Big {
        const CLASS: &'static str = "fixture.big";
        const OCSF: AuditOcsfClass = AuditOcsfClass::api_activity(1);
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
        builder.declare::<Big>().unwrap();
        builder.build()
    }

    #[test]
    fn details_need_a_declared_class_and_fit_16_kib() {
        let registry = registry();
        let detail = registry.detail(&Note { count: 3 }).unwrap();
        assert_eq!(detail.class().as_str(), "fixture.note");
        assert_eq!(
            registry
                .read::<Note>(detail.class(), detail.fields())
                .unwrap(),
            Note { count: 3 }
        );
        // The writer checks no schema: the owner's type keeps the shape.
        let mut extra = detail.fields().as_map().clone();
        extra.insert("more".into(), "text".into());
        assert!(
            registry
                .check_detail(detail.class(), &AuditDetailFields(extra))
                .is_ok()
        );
        let fits = Big {
            text: "x".repeat(MAX_DETAIL_BYTES - 16),
        };
        assert!(registry.detail(&fits).is_ok());
        let over = Big {
            text: "x".repeat(MAX_DETAIL_BYTES),
        };
        assert!(matches!(
            registry.detail(&over),
            Err(AuditDetailError::Invalid(_))
        ));
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
    fn declarations_carry_their_ocsf_class_and_stay_unique() {
        let mut builder = AuditRegistry::builder();
        assert!(matches!(
            builder.declare::<Platform>(),
            Err(AuditDetailError::Reserved(_))
        ));
        builder.declare::<Note>().unwrap();
        builder.declare::<Note>().unwrap();
        let class = AuditClass::parse(Note::CLASS).unwrap();
        let other =
            AuditDetailDeclaration::new(class.clone(), AuditOcsfClass::api_activity(4), None)
                .unwrap();
        assert_eq!(
            builder.declaration(other),
            Err(AuditDetailError::Collision(class.clone()))
        );
        assert!(matches!(
            AuditDetailDeclaration::new(
                class,
                AuditOcsfClass::API_ACTIVITY_OTHER,
                Some(Value::Bool(true))
            ),
            Err(AuditDetailError::Schema(_))
        ));
        let registry = builder.build();
        let note = registry
            .declaration(&AuditClass::parse(Note::CLASS).unwrap())
            .unwrap();
        assert_eq!(note.ocsf(), AuditOcsfClass::API_ACTIVITY_OTHER);
        assert!(note.schema().is_some());
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
            assert_eq!(declaration.ocsf().class_uid, 6003);
        }
    }
}
