//! PostgreSQL access for Veoveo components: the connection pool, the kernel and
//! identity migrations, process instances, singleton locks, change notifications,
//! registered servers' roles and schemas, the identity records, keyset pages and
//! batched maintenance. The design is in `docs/DATABASE.md`.

mod config;
mod error;
mod identity;
mod instance;
mod maintenance;
mod names;
mod notifications;
mod paging;
mod servers;
mod singleton;
mod store;
mod work_context;

pub use config::{StoreConfig, StoreConfigBuilder};
pub use error::{StoreConfigError, StoreError, is_connection_error};
pub use identity::{
    EnterpriseId, PlatformIdentity, PrincipalId, PrincipalKind, TenantId, WorkContextId,
    deterministic_enterprise_id, deterministic_principal_id, deterministic_tenant_id,
    deterministic_work_context_id,
};
pub use instance::{
    INSTANCE_GRACE, Instance, InstanceGrace, InstanceId, InstancePhase, InstanceState,
    PING_INTERVAL, PING_TIMEOUT, REAP_INTERVAL, RECONNECT_FIRST_DELAY, RECONNECT_MAX_DELAY,
    ReapRound,
};
pub use maintenance::{MaintenanceBatch, Pruned};
pub use names::{Channel, KERNEL_SCHEMAS, LoginName, MAX_SERVER_NAME, ServerSchema, SingletonName};
pub use notifications::{Change, ChangeOperation, ChangeSignal, ChangeStream};
pub use paging::{
    CursorError, CursorScope, KeysetCodec, KeysetCursor, Page, PageSize, PageSizeError,
};
pub use singleton::Singleton;
pub use store::{MIGRATOR, PlatformStore};
pub use work_context::{
    GrantPermission, GrantSubjectKind, WorkContextInitialGrantRecord, WorkContextMembershipLevel,
    WorkContextMembershipRuleRecord, WorkContextOutputPolicyRecord, WorkContextRecord,
};
