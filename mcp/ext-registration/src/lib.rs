//! The contract between a registering server and the Veoveo Gateway, described
//! normatively in `DESIGN.md` beside this crate.
//!
//! A server registers as an OAuth 2.0 dynamic client (RFC 7591, RFC 7592) whose client
//! metadata carries its identity, authorization declaration and module manifest, and
//! advertises the same declarations in MCP Discover as the `ai.veoveo/registration`,
//! `ai.veoveo/authorization` and `ai.veoveo/module` extensions. The Gateway forwards the
//! permissions it grants as an RFC 9396 authorization details entry. Servers build
//! every declaration from their own typed vocabularies; the Gateway checks them against
//! the namespace pinned to the server's key.
mod access;
pub mod authorization;
mod error;
pub mod grants;
pub mod jwk;
pub mod module;
pub mod oauth;
pub mod registration;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "mcp")]
pub mod mcp;
#[cfg(feature = "signing")]
pub mod statement;

pub use access::{Access, Risk};
pub use authorization::{
    AuthorizationBuilder, AuthorizationDeclaration, PermissionDeclaration, PermissionDefinition,
    RequirementTarget, Requirements,
};
pub use error::DeclarationError;
pub use grants::{PERMISSIONS_DETAIL_TYPE, PermissionsDetail, PermissionsDetailType};
pub use jwk::{Ed25519Jwk, Ed25519JwkSet, JwkKeyId, ServerKeySet, ServerPublicKey};
pub use module::{ModuleDeclaration, ModuleSummary};
pub use oauth::{
    AccessTokenTrust, AuthorizationServerMetadata, ClientInformation, ClientMetadata,
    ClientRegistrationRequest, ClientUpdateRequest, DatabaseGrant, DatabaseIdentifier, GrantType,
    RegistrationAccessToken, RegistrationErrorCode, RegistrationErrorResponse, RegistrationLease,
    ResponseType, SoftwareStatement, TokenEndpointAuthMethod, issuer_url,
};
pub use registration::{RegistrationDeclaration, ResourceIndicator, ServerEndpoint, ServerVersion};

/// MCP extension advertising [`RegistrationDeclaration`].
pub const REGISTRATION_EXTENSION_ID: &str = "ai.veoveo/registration";
/// MCP extension advertising [`AuthorizationDeclaration`].
pub const AUTHORIZATION_EXTENSION_ID: &str = "ai.veoveo/authorization";
/// MCP extension advertising [`ModuleSummary`].
pub const MODULE_EXTENSION_ID: &str = "ai.veoveo/module";
