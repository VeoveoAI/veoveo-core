//! The permissions the Gateway grants for one request, carried as an RFC 9396
//! authorization details entry in the RFC 9068 access token it forwards to a server.
use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use veoveo_types::{PermissionName, ServerSlug};

use crate::DeclarationError;

/// The RFC 9396 `type` of [`PermissionsDetail`].
pub const PERMISSIONS_DETAIL_TYPE: &str = "https://veoveo.ai/oauth/permissions";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, veoveo_types::Vocabulary)]
pub enum PermissionsDetailType {
    #[vocabulary(rename = "https://veoveo.ai/oauth/permissions")]
    Permissions,
}

/// `{"type": "https://veoveo.ai/oauth/permissions", "actions": ["time.read"]}`. The
/// RFC 9396 `actions` member lists the granted permissions, all in the namespace of
/// the token's audience.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PermissionsDetail {
    #[serde(rename = "type")]
    pub kind: PermissionsDetailType,
    pub actions: BTreeSet<PermissionName>,
}

impl PermissionsDetail {
    pub fn new(actions: BTreeSet<PermissionName>) -> Self {
        Self {
            kind: PermissionsDetailType::Permissions,
            actions,
        }
    }

    /// Every action must belong to `audience`'s namespace.
    pub fn check(&self, audience: &ServerSlug) -> Result<(), DeclarationError> {
        match self
            .actions
            .iter()
            .find(|permission| !permission.is_owned_by(audience))
        {
            Some(permission) => Err(DeclarationError::PermissionOutsideNamespace {
                permission: permission.clone(),
            }),
            None => Ok(()),
        }
    }
}
