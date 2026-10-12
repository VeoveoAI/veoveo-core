//! The contract between a registering server and the Veoveo Gateway, described
//! normatively in `DESIGN.md` beside this crate.
//!
//! A server declares itself through three MCP extensions, `ai.veoveo/registration`,
//! `ai.veoveo/authorization` and `ai.veoveo/module`, and registers them as OAuth 2.0
//! Dynamic Client Registration metadata (RFC 7591, RFC 7592). The Gateway forwards the
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
    ResponseType, SoftwareStatement, TokenEndpointAuthMethod,
};
pub use registration::{RegistrationDeclaration, ServerEndpoint, ServerVersion};

/// MCP extension and client metadata member for [`RegistrationDeclaration`].
pub const REGISTRATION_EXTENSION_ID: &str = "ai.veoveo/registration";
/// MCP extension and client metadata member for [`AuthorizationDeclaration`].
pub const AUTHORIZATION_EXTENSION_ID: &str = "ai.veoveo/authorization";
/// MCP extension for [`ModuleSummary`] and client metadata member for [`ModuleDeclaration`].
pub const MODULE_EXTENSION_ID: &str = "ai.veoveo/module";
/// Registration response member for [`RegistrationLease`].
pub const LEASE_MEMBER: &str = "ai.veoveo/lease";
/// Registration response member for [`AccessTokenTrust`].
pub const ACCESS_TOKENS_MEMBER: &str = "ai.veoveo/access-tokens";
/// Registration response member for [`DatabaseGrant`].
pub const DATABASE_MEMBER: &str = "ai.veoveo/database";
