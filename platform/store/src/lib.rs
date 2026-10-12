//! PostgreSQL access for Veoveo components: the connection pool, the kernel and
//! identity migrations, process instances, singleton locks, change notifications,
//! registered servers' roles and schemas, and the identity records. The design is in
//! `docs/DATABASE.md`.

mod config;
mod error;
#[cfg(feature = "gateway")]
pub mod gateway;
mod identity;
mod instance;
mod names;
mod notifications;
mod servers;
mod singleton;
mod store;
mod work_context;

pub use config::{StoreConfig, StoreConfigBuilder};
pub use error::{StoreConfigError, StoreError};
pub use identity::{
    EnterpriseId, PlatformIdentity, PrincipalId, PrincipalKind, TenantId, WorkContextId,
    deterministic_enterprise_id, deterministic_principal_id, deterministic_tenant_id,
    deterministic_work_context_id,
};
pub use instance::{Instance, InstanceId, PING_INTERVAL, PING_TIMEOUT, REAP_INTERVAL, ReapRound};
pub use names::{Channel, KERNEL_SCHEMAS, LoginName, MAX_SERVER_NAME, ServerSchema, SingletonName};
pub use notifications::{Change, ChangeOperation, ChangeSignal, ChangeStream};
pub use singleton::Singleton;
pub use store::{MIGRATOR, PlatformStore};
pub use work_context::{
    GrantPermission, GrantSubjectKind, WorkContextInitialGrantRecord, WorkContextMembershipLevel,
    WorkContextMembershipRuleRecord, WorkContextOutputPolicyRecord, WorkContextRecord,
};
