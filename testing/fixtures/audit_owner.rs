//! A fixture audit owner with a registered target and a declared detail class. Audit
//! tests use it to exercise owner registration and declarations without a real owner.
use veoveo_audit_contract::{
    AuditDetailError, AuditDetailType, AuditRegistryBuilder, AuditTargetError, AuditTargetOwner,
    AuditTargetRegistration,
};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(transparent)]
pub struct ItemId(uuid::Uuid);

impl ItemId {
    #[allow(
        dead_code,
        reason = "Each audit test selects fresh or literal identities"
    )]
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7())
    }

    #[allow(
        dead_code,
        reason = "Each audit test selects fresh or literal identities"
    )]
    pub fn parse(value: &str) -> Result<Self, uuid::Error> {
        uuid::Uuid::parse_str(value).map(Self)
    }

    #[allow(dead_code, reason = "Only some audit tests read the UUID")]
    pub fn as_uuid(&self) -> uuid::Uuid {
        self.0
    }
}

/// The fixture owner's target, `{"kind":"item","item":"<uuid>"}`.
#[derive(Debug, Clone, PartialEq, Eq, schemars::JsonSchema)]
#[schemars(transform = audit_target_schema)]
#[schemars(deny_unknown_fields)]
pub struct ItemAuditTarget {
    pub item: ItemId,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum ItemTargetWire {
    #[serde(rename = "item")]
    Item { item: ItemId },
}

impl serde::Serialize for ItemAuditTarget {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(&ItemTargetWire::Item { item: self.item }, serializer)
    }
}

impl<'de> serde::Deserialize<'de> for ItemAuditTarget {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match <ItemTargetWire as serde::Deserialize>::deserialize(deserializer)? {
            ItemTargetWire::Item { item } => Ok(Self { item }),
        }
    }
}

impl AuditTargetOwner for ItemAuditTarget {
    const KIND: &'static str = "item";
}

/// The fixture owner's detail, class `fixture.note`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Note {
    pub count: u32,
}

impl AuditDetailType for Note {
    const CLASS: &'static str = "fixture.note";
}

pub fn register_audit_target(
    builder: &mut AuditRegistryBuilder,
) -> Result<AuditTargetRegistration<ItemAuditTarget>, AuditTargetError> {
    builder.register_target::<ItemAuditTarget>()
}

#[allow(
    dead_code,
    reason = "Only some audit tests record the fixture's detail"
)]
pub fn declare_audit_detail(builder: &mut AuditRegistryBuilder) -> Result<(), AuditDetailError> {
    builder.declare::<Note>()
}

fn audit_target_schema(schema: &mut schemars::Schema) {
    let object = schema.as_object_mut().expect("item target object");
    object
        .get_mut("properties")
        .and_then(serde_json::Value::as_object_mut)
        .expect("item target properties")
        .insert(
            "kind".into(),
            serde_json::json!({"type":"string","const":"item"}),
        );
    object
        .get_mut("required")
        .and_then(serde_json::Value::as_array_mut)
        .expect("item target required fields")
        .push(serde_json::json!("kind"));
}
