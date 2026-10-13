//! Installation and caller names shared by every component.
use crate::IdentifierError;

#[doc = "Canonical name of a registered server: its schema, URI scheme and route name."]
#[veoveo_types::id(text(ServerSlugs))]
pub struct ServerSlug(String);

/// Names no server may register as its slug or URI scheme: the platform scheme, the
/// kernel's database schemas, the schemas PostgreSQL creates in every database and
/// common URI schemes registered with IANA. Slugs that start with `pg-` are reserved as
/// well, because PostgreSQL reserves schemas that start with `pg_`.
pub const RESERVED_SERVER_SLUGS: &[&str] = &[
    "veoveo",
    "kernel",
    "identity",
    "tasks",
    "audit",
    "public",
    "information-schema",
    "data",
    "file",
    "ftp",
    "http",
    "https",
    "mailto",
    "urn",
    "ws",
    "wss",
];

impl ServerSlug {
    /// Whether the slug is reserved for the platform, a kernel or PostgreSQL schema, or a
    /// registered scheme.
    pub fn is_reserved(&self) -> bool {
        RESERVED_SERVER_SLUGS.contains(&self.as_ref()) || self.as_str().starts_with("pg-")
    }
}
#[doc = "Registered OAuth client identifier."]
#[veoveo_types::id(text(ClientIds))]
pub struct OAuthClientId(String);

#[doc(hidden)]
pub struct ClientIds;
impl crate::IdProfile for ClientIds {
    type Error = IdentifierError;
    const PROFILE: crate::IdProfileSpec<Self::Error> = crate::IdProfileSpec {
        schema_id: Some(|_| concat!(module_path!(), "::OAuthClientId").into()),
        schema: crate::IdSchema::Owner {
            schema: client_schema,

            inline: false,
        },
        ..<crate::identifier_syntax::ClaimTextProfile as crate::IdProfile>::PROFILE
    };
}
fn client_schema(_: &mut schemars::SchemaGenerator, _: crate::IdMetadata) -> schemars::Schema {
    schemars::json_schema!({
        "type": "string",
        "description": "Registered OAuth client identifier.",
        "minLength": 1,
        "not": { "pattern": r"[\u0000-\u001f\u007f-\u009f]" }
    })
}

#[doc(hidden)]
pub struct ServerSlugs;
impl crate::IdProfile for ServerSlugs {
    type Error = IdentifierError;
    const PROFILE: crate::IdProfileSpec<Self::Error> =
        <crate::identifier_syntax::PathIdProfile as crate::IdProfile>::PROFILE;
    fn naming_profile(_: crate::IdMetadata) -> Option<crate::ScalarNaming> {
        Some(crate::ScalarNaming::builtin(
            crate::ScalarGrammar::ServerSlug,
        ))
    }
}
