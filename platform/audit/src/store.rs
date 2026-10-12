//! PostgreSQL persistence for the audit log: appends, partition-scoped reads, change
//! signals and the indexing windows. The sealer's writes live in [`crate::sealer`]
//! because they run on the connection that holds the sealer role.
use std::sync::Arc;
use veoveo_audit_contract::{
    AuditAppendError, AuditPartition, AuditReadScope, AuditTargetError, AuditTargetRegistry,
    AuditValidationError,
};
use veoveo_platform_store::{
    ChangeOperation, ChangeSignal, ChangeStream, Channel, PlatformStore, StoreError,
};

mod blocks;
pub(crate) mod delivery;
pub(crate) mod indexing;
mod live;
mod records;
mod row;
pub(crate) mod seal;

pub use blocks::AuditExportRange;
pub use live::{AuditLiveChange, AuditLiveStream};

/// SQLSTATE that `audit.append` and `audit.append_indexing` raise for a reused identity.
const IDENTITY_CONFLICT: &str = "VV501";
/// SQLSTATE that `audit.append_indexing` raises for a read outside its admission window.
const INDEXING_READ_EXPIRED: &str = "VV502";
/// SQLSTATE that the export functions raise when a block or intent disagrees.
const BLOCK_CONFLICT: &str = "VV503";
/// SQLSTATE that `audit.record_export` raises for a destination's recorded refusal.
const EXPORT_REJECTED: &str = "VV504";

#[derive(Debug, thiserror::Error)]
pub enum AuditStoreError {
    #[error(transparent)]
    Target(#[from] AuditTargetError),
    #[error(transparent)]
    Validation(#[from] AuditValidationError),
    #[error("audit partition access denied")]
    AccessDenied,
    #[error("audit row identity or document is inconsistent")]
    Integrity,
    #[error("audit batch exceeds the limit for this operation")]
    BatchLimit,
    #[error("an audit record identity already holds different content")]
    IdentityConflict,
    #[error("audit indexing read is outside its admission window")]
    IndexingReadExpired,
    #[error(
        "audit destination permanently rejected this delivery; configure a corrected destination before resuming export"
    )]
    ExportRejected,
    #[error("audit change notifications ended")]
    Disconnected,
    #[error("audit database operation failed: {0}")]
    Database(sqlx::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl From<sqlx::Error> for AuditStoreError {
    fn from(error: sqlx::Error) -> Self {
        let code = error
            .as_database_error()
            .and_then(|database| database.code())
            .map(|code| code.into_owned());
        match code.as_deref() {
            Some(IDENTITY_CONFLICT) => Self::IdentityConflict,
            Some(INDEXING_READ_EXPIRED) => Self::IndexingReadExpired,
            Some(BLOCK_CONFLICT) => Self::Integrity,
            Some(EXPORT_REJECTED) => Self::ExportRejected,
            _ => Self::Database(error),
        }
    }
}

impl From<AuditAppendError> for AuditStoreError {
    fn from(error: AuditAppendError) -> Self {
        match error {
            AuditAppendError::Target(error) => Self::Target(error),
            AuditAppendError::BatchLimit => Self::BatchLimit,
            AuditAppendError::IdentityConflict => Self::IdentityConflict,
            AuditAppendError::Database(error) => Self::Database(error),
        }
    }
}

impl AuditStoreError {
    /// A serialization failure or deadlock that a repeated transaction can resolve.
    pub(crate) fn retryable(&self) -> bool {
        let Self::Database(error) = self else {
            return false;
        };
        error
            .as_database_error()
            .and_then(|database| database.code())
            .is_some_and(|code| code == "40001" || code == "40P01")
    }
}

/// The audit log in the installation database, read and written with one immutable
/// target registry. Clones share the pool and the registry.
#[derive(Clone, Debug)]
pub struct AuditStore {
    store: PlatformStore,
    targets: Arc<AuditTargetRegistry>,
}

impl AuditStore {
    pub fn new(store: PlatformStore, targets: Arc<AuditTargetRegistry>) -> Self {
        Self { store, targets }
    }

    pub fn targets(&self) -> &AuditTargetRegistry {
        &self.targets
    }

    pub fn platform(&self) -> &PlatformStore {
        &self.store
    }

    pub(crate) fn pool(&self) -> &sqlx::PgPool {
        self.store.pool()
    }

    /// Listen on the audit channel and wait until the subscription is active, so a
    /// read that follows cannot miss a change committed after it.
    pub(crate) async fn subscribe(&self) -> Result<ChangeStream, AuditStoreError> {
        use futures::StreamExt;
        let mut changes = self.store.listen(vec![Channel::AUDIT]);
        loop {
            match changes.next().await {
                Some(Ok(ChangeSignal::Reconcile)) => return Ok(changes),
                Some(Ok(ChangeSignal::Changed(_))) => {}
                Some(Err(error)) => return Err(error.into()),
                None => return Err(AuditStoreError::Disconnected),
            }
        }
    }
}

pub(crate) fn admit(
    scope: &AuditReadScope,
    partition: &AuditPartition,
) -> Result<(), AuditStoreError> {
    if scope.permits(partition) {
        Ok(())
    } else {
        Err(AuditStoreError::AccessDenied)
    }
}

/// What one audit change signal reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuditSignal {
    /// Every watched state may have changed.
    Reconcile,
    /// A record committed with this identity.
    Record(uuid::Uuid),
    /// A block of the partition with this storage key was sealed.
    Sealed(String),
    /// Retention removed a block of the partition with this storage key.
    Retired(String),
    Other,
}

impl AuditSignal {
    pub(crate) fn decode(signal: ChangeSignal) -> Self {
        let change = match signal {
            ChangeSignal::Reconcile => return Self::Reconcile,
            ChangeSignal::Changed(change) => change,
        };
        let Some(key) = change.key else {
            return Self::Other;
        };
        match (change.table.as_str(), change.operation) {
            ("record", ChangeOperation::Insert) => {
                key.parse().map_or(Self::Reconcile, Self::Record)
            }
            ("block", ChangeOperation::Insert) => Self::Sealed(key),
            ("block", ChangeOperation::Delete) => Self::Retired(key),
            _ => Self::Other,
        }
    }
}
