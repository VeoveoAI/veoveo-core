//! The principal that submitted a Task and the authority it held at submission.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use veoveo_types::{
    DataLabelId, IdMetadata, IdProfile, IdProfileSpec, IdentifierError, InvocationAuthority,
    PrincipalId, TenantId, TokenIssuer, TokenSubject,
};

use crate::{PrincipalKind, TaskError, snapshot::is_valid_key};

/// The surface an owner submitted a Task through, such as the Gateway profile a call
/// arrived on. An owner reads a Task only through the surface it was created on. The
/// runtime treats the value as an opaque key of 1 to 256 bytes without control
/// characters; the protocol adapter that creates the Task chooses it.
#[veoveo_types::id(text(TaskSurfaces))]
pub struct TaskSurface(String);

fn validate_surface(value: &str, _: IdMetadata) -> Result<(), IdentifierError> {
    if is_valid_key(value) {
        Ok(())
    } else {
        Err(IdentifierError::new(
            value,
            "must be 1 to 256 bytes without control characters",
        ))
    }
}

#[doc(hidden)]
pub struct TaskSurfaces;
impl IdProfile for TaskSurfaces {
    type Error = IdentifierError;
    const PROFILE: IdProfileSpec<Self::Error> = IdProfileSpec::text(validate_surface);
}

/// The tenant of an owner without an explicit tenant claim.
pub(crate) const INSTALLATION_TENANT: &str = "installation";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskOwner {
    pub principal_key: PrincipalId,
    pub principal_kind: PrincipalKind,
    pub issuer: TokenIssuer,
    pub subject: TokenSubject,
    pub surface: TaskSurface,
    /// The explicit tenant claim; `None` for an installation-wide principal.
    pub tenant_key: Option<TenantId>,
    /// Data labels the principal holds. A reader of the Task must hold all of them.
    #[serde(default)]
    pub data_labels: BTreeSet<DataLabelId>,
    pub authority: InvocationAuthority,
}

impl TaskOwner {
    /// The owner's tenant: its explicit claim, or the installation tenant.
    pub fn tenant(&self) -> TenantId {
        self.tenant_key.clone().unwrap_or_else(|| {
            TenantId::parse(INSTALLATION_TENANT).expect("the installation tenant is admitted")
        })
    }

    /// Reject an owner whose Work Context belongs to another tenant.
    pub(crate) fn check_authority(&self) -> Result<(), TaskError> {
        if self.authority.tenant != self.tenant() {
            return Err(TaskError::InvalidAuthority(
                "task owner and Work Context belong to different tenants".into(),
            ));
        }
        Ok(())
    }
}

/// Bind values that select one owner's Tasks; see `queries/owner/get.sql`.
pub(crate) struct OwnerScope {
    pub server: String,
    pub tenant: String,
    pub principal: String,
    pub surface: String,
    pub owner_tenant: Option<String>,
    pub labels: Vec<String>,
    pub task_types: Option<Vec<String>>,
    pub work_context: Option<String>,
}
