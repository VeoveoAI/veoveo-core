//! A fixture owner Audit target with the frozen Computer record wire. Audit tests use it
//! to exercise owner registration and frozen record hashes without the Computers crate.
use veoveo_audit_contract::{
    AuditTargetError, AuditTargetOwner, AuditTargetRegistration, AuditTargetRegistryBuilder,
};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(transparent)]
pub struct ComputerId(uuid::Uuid);

impl ComputerId {
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

#[derive(Debug, Clone, PartialEq, Eq, schemars::JsonSchema)]
#[schemars(transform = audit_target_schema)]
#[schemars(deny_unknown_fields)]
pub struct ComputerAuditTarget {
    pub computer: ComputerId,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum ComputerTargetWire {
    #[serde(rename = "computer")]
    Computer { computer: ComputerId },
}

impl serde::Serialize for ComputerAuditTarget {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(
            &ComputerTargetWire::Computer {
                computer: self.computer,
            },
            serializer,
        )
    }
}

impl<'de> serde::Deserialize<'de> for ComputerAuditTarget {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match <ComputerTargetWire as serde::Deserialize>::deserialize(deserializer)? {
            ComputerTargetWire::Computer { computer } => Ok(Self { computer }),
        }
    }
}

impl AuditTargetOwner for ComputerAuditTarget {
    const KIND: &'static str = "computer";
}

pub fn register_audit_target(
    builder: &mut AuditTargetRegistryBuilder,
) -> Result<AuditTargetRegistration<ComputerAuditTarget>, AuditTargetError> {
    builder.register::<ComputerAuditTarget>()
}

fn audit_target_schema(schema: &mut schemars::Schema) {
    let object = schema.as_object_mut().expect("Computer target object");
    object
        .get_mut("properties")
        .and_then(serde_json::Value::as_object_mut)
        .expect("Computer target properties")
        .insert(
            "kind".into(),
            serde_json::json!({"type":"string","const":"computer"}),
        );
    object
        .get_mut("required")
        .and_then(serde_json::Value::as_array_mut)
        .expect("Computer target required fields")
        .push(serde_json::json!("kind"));
}
