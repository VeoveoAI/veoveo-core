//! The audit log in PostgreSQL: group-committed and transactional appends,
//! partition-scoped reads and change streams, the singleton sealer with export and
//! retention, and signed-block verification.
pub use veoveo_audit_contract::*;
mod store;
pub use store::{AuditExportRange, AuditLiveChange, AuditLiveStream, AuditStore, AuditStoreError};
mod writer;
pub use writer::{AuditShutdownError, AuditWriteError, AuditWriter};
pub mod integrity;
pub mod sealer;
mod service;
pub use service::{AuditHealth, AuditHealthState, AuditService, AuditServiceError};

pub mod export;
