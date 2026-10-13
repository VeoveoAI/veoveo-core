//! Typed audit records shared by persistence, producers and readers.
//! This crate has no protocol runtime, HTTP client or async runtime. Its `postgres`
//! feature adds the sqlx bindings and `AuditTransactionWrite`, which appends records
//! inside a caller's PostgreSQL transaction.
mod activity;
pub use activity::*;
mod class;
pub use class::*;
mod decoder;
pub use decoder::*;
mod destination;
pub use destination::*;
mod detail;
pub use detail::*;
mod ids;
pub use ids::*;
mod integrity;
pub use integrity::*;
mod model;
pub use model::*;
#[cfg(feature = "postgres")]
mod postgres;
#[cfg(feature = "postgres")]
pub use postgres::*;
mod query;
pub use query::*;
mod reader;
pub use reader::*;
mod registry;
pub use registry::*;
mod stored;
pub use stored::*;

/// The audit domain owns this scope; protocol layers only carry its validated name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, veoveo_types::Vocabulary)]
#[vocabulary(scope)]
pub enum AuditScope {
    #[vocabulary(rename = "audit:read")]
    Read,
}

#[derive(Debug, Clone, Copy)]
pub enum InstallationAuditRole {
    Administrator,
    Auditor,
}
impl InstallationAuditRole {
    pub fn id(self) -> &'static veoveo_types::RoleId {
        use std::sync::LazyLock;
        static ADMIN: LazyLock<veoveo_types::RoleId> =
            LazyLock::new(|| veoveo_types::RoleId::parse("administrator").expect("declared role"));
        static AUDITOR: LazyLock<veoveo_types::RoleId> =
            LazyLock::new(|| veoveo_types::RoleId::parse("auditor").expect("declared role"));
        match self {
            Self::Administrator => &ADMIN,
            Self::Auditor => &AUDITOR,
        }
    }
}
