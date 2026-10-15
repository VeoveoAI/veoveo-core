//! A fixture audit owner with a declared detail class and its own resource addresses.
//! Audit tests use it to exercise owner declarations and targets without a real owner.
use veoveo_audit_contract::{AuditDetailError, AuditDetailType, AuditRegistryBuilder, AuditTarget};

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

/// The fixture owner's address of an item, `fixture://items/<uuid>`.
#[allow(dead_code, reason = "Only some audit tests name a target")]
pub fn item_target(item: ItemId) -> AuditTarget {
    AuditTarget::parse(format!("fixture://items/{}", item.as_uuid())).expect("an item address")
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

#[allow(
    dead_code,
    reason = "Only some audit tests record the fixture's detail"
)]
pub fn declare_audit_detail(builder: &mut AuditRegistryBuilder) -> Result<(), AuditDetailError> {
    builder.declare::<Note>()
}
