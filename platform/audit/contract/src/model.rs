use crate::*;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, net::IpAddr};
use veoveo_types::{
    DataLabelId, OAuthClientId, PolicyVersion, PrincipalId, ResourceUri, ScopeName, ServerSlug,
    TaskId, TenantId, WorkContextId,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    content = "tenant",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AuditPartition {
    Installation,
    Tenant(TenantId),
}
impl AuditPartition {
    pub fn storage_key(&self) -> String {
        match self {
            Self::Installation => "installation".into(),
            Self::Tenant(id) => format!("tenant:{id}"),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuditPrincipalKind {
    User,
    Service,
    Capability,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditActor {
    pub principal: PrincipalId,
    pub kind: AuditPrincipalKind,
    pub tenant: Option<TenantId>,
    pub oauth_client: Option<OAuthClientId>,
    pub delegating_principal: Option<PrincipalId>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditAuthority {
    pub work_context: Option<WorkContextId>,
    pub policy_revision: Option<PolicyVersion>,
    pub scopes: BTreeSet<ScopeName>,
    pub data_labels: BTreeSet<DataLabelId>,
}
/// What a record is about: a platform object, any resource by its URI, or an owner's
/// registered target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuditTarget {
    Resource {
        uri: ResourceUri,
    },
    Server {
        server: ServerSlug,
    },
    Task {
        task: TaskId,
    },
    Principal {
        tenant: TenantId,
        principal: PrincipalId,
    },
    WorkContext {
        tenant: TenantId,
        context: WorkContextId,
    },
    AuditLog {
        partition: AuditPartition,
    },
    Installation,
    #[serde(untagged)]
    Extension(crate::AdmittedAuditTarget),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CoreAuditTarget {
    Resource {
        uri: ResourceUri,
    },
    Server {
        server: ServerSlug,
    },
    Task {
        task: TaskId,
    },
    Principal {
        tenant: TenantId,
        principal: PrincipalId,
    },
    WorkContext {
        tenant: TenantId,
        context: WorkContextId,
    },
    AuditLog {
        partition: AuditPartition,
    },
    Installation,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditRequest {
    pub id: AuditRequestId,
    pub trace_id: AuditTraceId,
    pub span_id: AuditSpanId,
    pub source_ip: Option<IpAddr>,
}
/// Verified attribution retained by a domain operation for its lifecycle records.
/// Producers construct this from their authenticated protocol context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditContext {
    pub actor: AuditActor,
    pub authority: AuditAuthority,
    pub request: AuditRequest,
}
impl AuditContext {
    pub fn draft(
        &self,
        target: AuditTarget,
        detail: AuditDetail,
        outcome: AuditOutcome,
        reason: AuditReason,
    ) -> Result<AuditDraft, AuditValidationError> {
        AuditDraft::builder(self.request.clone(), target, detail, outcome, reason)
            .actor(self.actor.clone())
            .authority(self.authority.clone())
            .build()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AuditRecordFormat {
    #[serde(rename = "https://veoveo.ai/formats/audit-record")]
    AuditRecord,
}

/// Immutable constructor-checked action. The database supplies recorded_at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(into = "AuditDraftWire")]
pub struct AuditDraft(AuditDraftWire);
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuditDraftWire {
    pub(crate) id: AuditRecordId,
    pub(crate) format: AuditRecordFormat,
    pub(crate) partition: AuditPartition,
    pub(crate) actor: Option<AuditActor>,
    pub(crate) authority: AuditAuthority,
    pub(crate) target: AuditTarget,
    pub(crate) class: AuditClass,
    pub(crate) detail: AuditDetailFields,
    pub(crate) outcome: AuditOutcome,
    pub(crate) reason: AuditReason,
    pub(crate) request: AuditRequest,
    pub(crate) occurred_at: DateTime<Utc>,
    pub(crate) latency_ms: Option<u64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuditValidationError {
    #[error("audit counters must fit the exact I-JSON integer range")]
    CounterRange,
    #[error("audit partition must match the authenticated actor's tenant")]
    Partition,
    #[error("an accepted outcome cannot carry a denial reason")]
    Outcome,
    #[error("audit cursor belongs to a different partition")]
    Cursor,
    #[error("audit time range must increase")]
    TimeRange,
    #[error("daily audit bounds and cursor must use midnight UTC")]
    DayBoundary,
    #[error("daily audit counts cover at most 366 days")]
    DailySpan,
    #[error("audit page size must be between 1 and 1000")]
    PageSize,
}
impl TryFrom<AuditDraftWire> for AuditDraft {
    type Error = AuditValidationError;
    fn try_from(wire: AuditDraftWire) -> Result<Self, Self::Error> {
        let partition = wire
            .actor
            .as_ref()
            .and_then(|a| a.tenant.clone())
            .map_or(AuditPartition::Installation, AuditPartition::Tenant);
        if partition != wire.partition {
            return Err(AuditValidationError::Partition);
        }
        let accepted = matches!(
            wire.outcome,
            AuditOutcome::Allowed | AuditOutcome::Succeeded
        );
        if accepted != (wire.reason == AuditReason::Accepted) {
            return Err(AuditValidationError::Outcome);
        }
        const MAX_COUNTER: u64 = (1u64 << 53) - 1;
        if wire.latency_ms.is_some_and(|value| value > MAX_COUNTER) {
            return Err(AuditValidationError::CounterRange);
        }
        Ok(Self(wire))
    }
}
impl From<AuditDraft> for AuditDraftWire {
    fn from(draft: AuditDraft) -> Self {
        draft.0
    }
}
impl AuditDraft {
    pub fn builder(
        request: AuditRequest,
        target: AuditTarget,
        detail: AuditDetail,
        outcome: AuditOutcome,
        reason: AuditReason,
    ) -> AuditDraftBuilder {
        let (class, detail) = detail.into_parts();
        AuditDraftBuilder(AuditDraftWire {
            id: AuditRecordId::new(),
            format: AuditRecordFormat::AuditRecord,
            partition: AuditPartition::Installation,
            actor: None,
            authority: AuditAuthority::default(),
            target,
            class,
            detail,
            outcome,
            reason,
            request,
            occurred_at: Utc::now(),
            latency_ms: None,
        })
    }
    pub fn id(&self) -> AuditRecordId {
        self.0.id
    }
    pub fn partition(&self) -> &AuditPartition {
        &self.0.partition
    }
    pub fn actor(&self) -> Option<&AuditActor> {
        self.0.actor.as_ref()
    }
    pub fn authority(&self) -> &AuditAuthority {
        &self.0.authority
    }
    pub fn target(&self) -> &AuditTarget {
        &self.0.target
    }
    pub fn class(&self) -> &AuditClass {
        &self.0.class
    }
    pub fn detail(&self) -> &AuditDetailFields {
        &self.0.detail
    }
    pub fn outcome(&self) -> AuditOutcome {
        self.0.outcome
    }
    pub fn reason(&self) -> AuditReason {
        self.0.reason
    }
    pub fn request(&self) -> &AuditRequest {
        &self.0.request
    }
    pub fn occurred_at(&self) -> DateTime<Utc> {
        self.0.occurred_at
    }
    pub fn latency_ms(&self) -> Option<u64> {
        self.0.latency_ms
    }
}
pub struct AuditDraftBuilder(AuditDraftWire);
impl AuditDraftBuilder {
    pub fn actor(mut self, actor: AuditActor) -> Self {
        self.0.partition = actor
            .tenant
            .clone()
            .map_or(AuditPartition::Installation, AuditPartition::Tenant);
        self.0.actor = Some(actor);
        self
    }
    pub fn authority(mut self, authority: AuditAuthority) -> Self {
        self.0.authority = authority;
        self
    }
    pub fn identity(mut self, id: AuditRecordId) -> Self {
        self.0.id = id;
        self
    }
    pub fn occurred_at(mut self, at: DateTime<Utc>) -> Self {
        self.0.occurred_at = at;
        self
    }
    pub fn latency_ms(mut self, milliseconds: u64) -> Self {
        self.0.latency_ms = Some(milliseconds);
        self
    }
    pub fn build(self) -> Result<AuditDraft, AuditValidationError> {
        self.0.try_into()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditRecord {
    pub draft: AuditDraft,
    pub recorded_at: DateTime<Utc>,
}

impl AuditRequest {
    /// Start a new non-HTTP operation. HTTP ingress uses its admitted trace context.
    pub fn background() -> Self {
        let trace = uuid::Uuid::new_v4().simple().to_string();
        let span = uuid::Uuid::new_v4().simple().to_string()[..16].to_owned();
        Self {
            id: AuditRequestId::new(),
            trace_id: AuditTraceId::parse(trace).expect("UUID trace"),
            span_id: AuditSpanId::parse(span).expect("UUID span"),
            source_ip: None,
        }
    }
}

impl From<CoreAuditTarget> for AuditTarget {
    fn from(value: CoreAuditTarget) -> Self {
        match value {
            CoreAuditTarget::Resource { uri } => Self::Resource { uri },
            CoreAuditTarget::Server { server } => Self::Server { server },
            CoreAuditTarget::Task { task } => Self::Task { task },
            CoreAuditTarget::Principal { tenant, principal } => {
                Self::Principal { tenant, principal }
            }
            CoreAuditTarget::WorkContext { tenant, context } => {
                Self::WorkContext { tenant, context }
            }
            CoreAuditTarget::AuditLog { partition } => Self::AuditLog { partition },
            CoreAuditTarget::Installation => Self::Installation,
        }
    }
}

impl JsonSchema for AuditTarget {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "AuditTarget".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        CoreAuditTarget::json_schema(generator)
    }
}
