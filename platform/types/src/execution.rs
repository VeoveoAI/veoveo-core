//! Identity of execution selections and instances, independent of transport or runtime.

use uuid::Uuid;

use crate::IdentifierError;

/// Stable identity of an execution environment, independent of its label or current composition.
///
/// The environment owner resolves selection and access. This is not a tenant or business
/// work context, and possession of this identifier grants no authority.
///
/// ```compile_fail
/// use veoveo_types::{EnvironmentId, WorkContextId};
/// fn select(_: EnvironmentId) {}
/// select(WorkContextId::parse("project").unwrap());
/// ```
#[veoveo_types::id(uuid(ExecutionIds), fresh)]
pub struct EnvironmentId(Uuid);

/// Identity of one admitted deployment of a logical component.
///
/// Distinct deployments can implement the same logical component simultaneously. The
/// routing owner binds this identifier to an admitted instance; it is not an endpoint,
/// process identifier or permission to register in a component's namespace.
///
/// ```compile_fail
/// use veoveo_types::{DeploymentId, EnvironmentId};
/// fn retain(_: DeploymentId) {}
/// retain(EnvironmentId::new());
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
