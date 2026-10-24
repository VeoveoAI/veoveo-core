//! Identity of deployments and builds, independent of transport or runtime.

use uuid::Uuid;

use crate::IdentifierError;

/// Identity of one admitted deployment of a logical component.
///
/// Distinct deployments can implement the same logical component simultaneously. The
/// routing owner binds this identifier to an admitted instance; it is not an endpoint,
/// process identifier or permission to register in a component's namespace.
///
/// ```compile_fail
/// use veoveo_types::{BuildId, DeploymentId};
/// fn retain(_: DeploymentId) {}
/// retain(BuildId::new());
/// ```
#[veoveo_types::id(uuid(ExecutionIds), fresh)]
pub struct DeploymentId(Uuid);

/// Unique identity of one build attempt and its immutable outputs.
///
/// The builder allocates a new identity even when rebuilding unchanged source. Runtime,
/// toolchain, options and output provenance belong to the build record, not this UUID.
/// Never reuse a build's executable path for different bytes.
#[veoveo_types::id(uuid(ExecutionIds), fresh)]
pub struct BuildId(Uuid);

#[doc(hidden)]
pub struct ExecutionIds;

impl crate::IdProfile for ExecutionIds {
    type Error = IdentifierError;
    const PROFILE: crate::IdProfileSpec<Self::Error> =
        crate::IdProfileSpec::generated_uuid(crate::UuidGrammar::canonical(&[7]), |value, _, _| {
            IdentifierError::new(value, "expected a canonical lowercase UUIDv7")
        })
        .owner_schema(execution_id_schema, false);
}

fn execution_id_schema(
    _: &mut schemars::SchemaGenerator,
    _: crate::IdMetadata,
) -> schemars::Schema {
    schemars::json_schema!({
        "type": "string",
        "format": "uuid",
        "pattern": "^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"
    })
}
