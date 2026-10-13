//! The Gateway's state in the `gateway` schema: hosted OAuth requests and codes,
//! refresh-token families, JWT revocations, replay identifiers, published control-plane
//! revisions, the authority an Artifact upload binds to and the cleanup of expired Task
//! routes. The Gateway runs its own statements for registrations and for creating and
//! reading Task routes against the same schema.

mod authorization;
mod control;
mod refresh;
mod replay;
mod revocations;
mod task_routes;

pub use control::{ArtifactUploadAuthority, ControlObject, ControlPublication};
pub use refresh::{
    NewRefreshFamily, NewRefreshToken, RefreshBinding, RefreshDeliveryEnvelope, RefreshFamily,
    RefreshPrincipal, RefreshRetention, RefreshRevocation, RefreshRotation, RefreshToken,
};
pub use replay::{ReplayIdentifier, ReplayKind};

use std::future::Future;

use sqlx::PgConnection;

use crate::StoreError;

/// Rows another owner records with a gateway change, such as the change's audit
/// records. The store appends them on the change's own connection before it commits,
/// and an error rolls the change back.
pub trait TransactionAppend: Sync {
    fn append(
        &self,
        connection: &mut PgConnection,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// Admit a stored column through its owning type. A row that no longer admits is an
/// error, never a default.
fn admitted<T, E>(column: &'static str, value: Result<T, E>) -> Result<T, StoreError> {
    value.map_err(|_| StoreError::InvalidStoredValue { column })
}
