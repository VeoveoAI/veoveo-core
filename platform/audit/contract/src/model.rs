use crate::*;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use veoveo_types::{
    IdMetadata, IdProfile, IdProfileSpec, IdentifierError, PrincipalId, ServerSlug, TaskId,
    TenantId, WorkContextId,
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

/// Who acted: a principal in its tenant and Work Context. A record without an actor is
/// the system's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditActor {
    pub principal: PrincipalId,
    pub kind: AuditPrincipalKind,
    pub tenant: Option<TenantId>,
    pub work_context: Option<WorkContextId>,
}

/// The server that acted for the actor through the Gateway, and the Task of that server
/// the call came from. OCSF names it as the actor's application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditVia {
    pub server: ServerSlug,
    pub task: Option<TaskId>,
}

/// Where the record came from: the request, its W3C trace, the peer and the Task the
/// action ran in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditRequest {
    pub id: AuditRequestId,
    pub trace_id: AuditTraceId,
    pub span_id: AuditSpanId,
    pub source_ip: Option<IpAddr>,
    pub task: Option<TaskId>,
}

/// Verified attribution retained by a domain operation for its lifecycle records.
/// Producers construct this from their authenticated protocol context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditContext {
    pub actor: AuditActor,
    pub via: Option<AuditVia>,
    pub request: AuditRequest,
}
impl AuditContext {
    /// A record of this context's actor, acting server and request.
    pub fn draft(&self, detail: AuditDetail, outcome: AuditOutcome) -> AuditDraftBuilder {
        let builder =
            AuditDraft::builder(self.request.clone(), detail, outcome).actor(self.actor.clone());
        match &self.via {
            Some(via) => builder.via(via.clone()),
            None => builder,
        }
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
    pub(crate) via: Option<AuditVia>,
    pub(crate) target: Option<AuditTarget>,
    pub(crate) class: AuditClass,
    pub(crate) detail: AuditDetailFields,
    pub(crate) outcome: AuditOutcome,
    pub(crate) reason: Option<AuditReason>,
    pub(crate) request: AuditRequest,
    pub(crate) occurred_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuditValidationError {
    #[error("audit partition must match the authenticated actor's tenant")]
    Partition,
    #[error("an audit detail holds at most 16 KiB of JSON")]
    DetailSize,
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
        check_envelope(&wire.partition, wire.actor.as_ref(), wire.detail.as_map())?;
        Ok(Self(wire))
    }
}

/// The envelope rules every record satisfies, whether its owner's types are known or
/// only its stored document: the partition follows the actor's tenant and the detail
/// holds at most [`MAX_DETAIL_BYTES`] of JSON.
pub(crate) fn check_envelope(
    partition: &AuditPartition,
    actor: Option<&AuditActor>,
    detail: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), AuditValidationError> {
    let expected = actor
        .and_then(|actor| actor.tenant.clone())
        .map_or(AuditPartition::Installation, AuditPartition::Tenant);
    if &expected != partition {
        return Err(AuditValidationError::Partition);
    }
    if detail_size(detail) > MAX_DETAIL_BYTES {
        return Err(AuditValidationError::DetailSize);
    }
    Ok(())
}
impl From<AuditDraft> for AuditDraftWire {
    fn from(draft: AuditDraft) -> Self {
        draft.0
    }
}
impl AuditDraft {
    /// A record of `detail`'s class with `outcome`. The builder adds the actor, the
    /// acting server, the target, the reason, the identity and the occurrence time.
    pub fn builder(
        request: AuditRequest,
        detail: AuditDetail,
        outcome: AuditOutcome,
    ) -> AuditDraftBuilder {
        let (class, detail) = detail.into_parts();
        AuditDraftBuilder(AuditDraftWire {
            id: AuditRecordId::new(),
            format: AuditRecordFormat::AuditRecord,
            partition: AuditPartition::Installation,
            actor: None,
            via: None,
            target: None,
            class,
            detail,
            outcome,
            reason: None,
            request,
            occurred_at: Utc::now(),
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
    pub fn via(&self) -> Option<&AuditVia> {
        self.0.via.as_ref()
    }
    pub fn target(&self) -> Option<&AuditTarget> {
        self.0.target.as_ref()
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
    pub fn reason(&self) -> Option<&AuditReason> {
        self.0.reason.as_ref()
    }
    pub fn request(&self) -> &AuditRequest {
        &self.0.request
    }
    pub fn occurred_at(&self) -> DateTime<Utc> {
        self.0.occurred_at
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
    pub fn via(mut self, via: AuditVia) -> Self {
        self.0.via = Some(via);
        self
    }
    /// What the record is about, as its owner's URI. A Task is `urn:uuid:<id>`.
    pub fn target(mut self, target: AuditTarget) -> Self {
        self.0.target = Some(target);
        self
    }
    pub fn reason(mut self, reason: AuditReason) -> Self {
        self.0.reason = Some(reason);
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
            task: None,
        }
    }
}

/// The longest target, in bytes.
pub const MAX_TARGET_BYTES: usize = 2048;

/// What a record is about: an absolute URI (RFC 3986) of at most 2048 bytes, in its
/// owner's own address form, such as `artifact://artifacts/{id}`. A Task is the RFC 9562
/// UUID URN of its identity, from [`task_target`].
#[cfg_attr(
    feature = "postgres",
    veoveo_types::id(text(AuditTargetProfile), postgres)
)]
#[cfg_attr(not(feature = "postgres"), veoveo_types::id(text(AuditTargetProfile)))]
pub struct AuditTarget(String);

fn validate_target(value: &str, _: IdMetadata) -> Result<(), IdentifierError> {
    if value.len() <= MAX_TARGET_BYTES && iri_string::types::UriAbsoluteStr::new(value).is_ok() {
        Ok(())
    } else {
        Err(IdentifierError::new(
            value,
            "must be an absolute URI without a fragment, at most 2048 bytes",
        ))
    }
}

#[doc(hidden)]
pub struct AuditTargetProfile;
impl IdProfile for AuditTargetProfile {
    type Error = IdentifierError;
    const PROFILE: IdProfileSpec<Self::Error> = IdProfileSpec::text(validate_target);
}

/// The target an audit record names a Task by: the RFC 9562 UUID URN of its identity.
pub fn task_target(task: TaskId) -> AuditTarget {
    AuditTarget::parse(format!("urn:uuid:{}", task.as_uuid()))
        .expect("a UUID URN is an absolute URI")
}
