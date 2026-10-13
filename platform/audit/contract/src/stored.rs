//! Records as the audit log stores them. Sealing, the hash chain, block signing and
//! export work on the stored document, so they never need another owner's target codec
//! or detail declaration; decoding an owner's target and detail is a reader's concern.
use crate::{
    AuditActor, AuditAuthority, AuditClass, AuditDecodeError, AuditOutcome, AuditPartition,
    AuditReason, AuditRecord, AuditRecordFormat, AuditRecordId, AuditRequest, AuditTargetError,
    model::check_envelope,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// What sealing and verification read from a record. A record's leaf hash covers its
/// canonical serialization, which is the same for a decoded [`AuditRecord`] and the
/// [`AuditStoredRecord`] it was stored as.
pub trait AuditSealedRecord: Serialize {
    fn record_id(&self) -> AuditRecordId;
    fn record_partition(&self) -> &AuditPartition;
    fn record_occurred_at(&self) -> DateTime<Utc>;
    fn record_recorded_at(&self) -> DateTime<Utc>;
}

impl AuditSealedRecord for AuditRecord {
    fn record_id(&self) -> AuditRecordId {
        self.draft.id()
    }
    fn record_partition(&self) -> &AuditPartition {
        self.draft.partition()
    }
    fn record_occurred_at(&self) -> DateTime<Utc> {
        self.draft.occurred_at()
    }
    fn record_recorded_at(&self) -> DateTime<Utc> {
        self.recorded_at
    }
}

/// The envelope members of a stored draft. The target and the detail stay JSON: their
/// shapes belong to the owners that registered and declared them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDraft {
    id: AuditRecordId,
    #[allow(dead_code, reason = "admission requires the record format")]
    format: AuditRecordFormat,
    partition: AuditPartition,
    actor: Option<AuditActor>,
    authority: AuditAuthority,
    target: Map<String, Value>,
    class: AuditClass,
    detail: Map<String, Value>,
    outcome: AuditOutcome,
    reason: AuditReason,
    request: AuditRequest,
    occurred_at: DateTime<Utc>,
    latency_ms: Option<u64>,
}

/// A record as stored: the draft document exactly as its producer appended it, admitted
/// for its envelope only, and the time the database recorded it. Its serialization is
/// the stored document, so its canonical bytes and leaf hash equal those of the decoded
/// record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditStoredRecord {
    document: Map<String, Value>,
    draft: StoredDraft,
    recorded_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct StoredRecordWire<'a> {
    draft: &'a Map<String, Value>,
    recorded_at: &'a DateTime<Utc>,
}

impl Serialize for AuditStoredRecord {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        StoredRecordWire {
            draft: &self.document,
            recorded_at: &self.recorded_at,
        }
        .serialize(serializer)
    }
}

impl AuditStoredRecord {
    /// Admit a stored draft document. The envelope must satisfy every record's rules,
    /// the target must be an object that names its kind and the detail an object; their
    /// owner-defined contents are not decoded.
    pub fn admit(draft: Value, recorded_at: DateTime<Utc>) -> Result<Self, AuditDecodeError> {
        let Value::Object(document) = draft else {
            return Err(AuditDecodeError::Target(AuditTargetError::Invalid));
        };
        let parsed: StoredDraft = serde_json::from_value(Value::Object(document.clone()))?;
        if !parsed.target.get("kind").is_some_and(Value::is_string) {
            return Err(AuditTargetError::Invalid.into());
        }
        check_envelope(
            &parsed.partition,
            parsed.actor.as_ref(),
            parsed.outcome,
            parsed.reason,
            parsed.latency_ms,
        )?;
        Ok(Self {
            document,
            draft: parsed,
            recorded_at,
        })
    }

    pub fn id(&self) -> AuditRecordId {
        self.draft.id
    }
    pub fn partition(&self) -> &AuditPartition {
        &self.draft.partition
    }
    pub fn actor(&self) -> Option<&AuditActor> {
        self.draft.actor.as_ref()
    }
    pub fn authority(&self) -> &AuditAuthority {
        &self.draft.authority
    }
    /// The target's kind, as its owner registered it.
    pub fn target_kind(&self) -> &str {
        self.draft
            .target
            .get("kind")
            .and_then(Value::as_str)
            .expect("an admitted target names its kind")
    }
    pub fn target(&self) -> &Map<String, Value> {
        &self.draft.target
    }
    pub fn class(&self) -> &AuditClass {
        &self.draft.class
    }
    pub fn detail(&self) -> &Map<String, Value> {
        &self.draft.detail
    }
    pub fn outcome(&self) -> AuditOutcome {
        self.draft.outcome
    }
    pub fn reason(&self) -> AuditReason {
        self.draft.reason
    }
    pub fn request(&self) -> &AuditRequest {
        &self.draft.request
    }
    pub fn occurred_at(&self) -> DateTime<Utc> {
        self.draft.occurred_at
    }
    pub fn latency_ms(&self) -> Option<u64> {
        self.draft.latency_ms
    }
    pub fn recorded_at(&self) -> DateTime<Utc> {
        self.recorded_at
    }
}

impl AuditSealedRecord for AuditStoredRecord {
    fn record_id(&self) -> AuditRecordId {
        self.id()
    }
    fn record_partition(&self) -> &AuditPartition {
        self.partition()
    }
    fn record_occurred_at(&self) -> DateTime<Utc> {
        self.occurred_at()
    }
    fn record_recorded_at(&self) -> DateTime<Utc> {
        self.recorded_at
    }
}

impl AuditRecord {
    /// The record as the audit log stores it.
    pub fn stored(&self) -> AuditStoredRecord {
        let draft = serde_json::to_value(&self.draft).expect("an admitted draft serializes");
        AuditStoredRecord::admit(draft, self.recorded_at)
            .expect("an admitted draft satisfies the stored envelope")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AuditDraft, AuditRegistry, AuditRequest, AuditTarget, platform};

    fn record() -> AuditRecord {
        AuditRecord {
            draft: AuditDraft::builder(
                AuditRequest::background(),
                AuditTarget::Installation,
                AuditRegistry::platform()
                    .detail(&platform::TaskUpdate {})
                    .unwrap(),
                AuditOutcome::Succeeded,
                AuditReason::Accepted,
            )
            .build()
            .unwrap(),
            recorded_at: Utc::now(),
        }
    }

    #[test]
    fn a_stored_record_serializes_as_its_decoded_record() {
        let record = record();
        let stored = record.stored();
        assert_eq!(
            serde_json::to_value(&stored).unwrap(),
            serde_json::to_value(&record).unwrap()
        );
        assert_eq!(stored.id(), record.draft.id());
        assert_eq!(stored.class().as_str(), "task.update");
        assert_eq!(stored.target_kind(), "installation");
    }

    #[test]
    fn unknown_owner_targets_and_details_admit_while_the_envelope_is_checked() {
        let mut draft = serde_json::to_value(&record().draft).unwrap();
        draft["target"] = serde_json::json!({"kind": "shelf", "shelf": "a-1"});
        draft["class"] = "inventory.moved".into();
        draft["detail"] = serde_json::json!({"count": 3});
        let stored = AuditStoredRecord::admit(draft.clone(), Utc::now()).unwrap();
        assert_eq!(stored.target_kind(), "shelf");
        assert_eq!(stored.detail()["count"], 3);

        let mut kindless = draft.clone();
        kindless["target"] = serde_json::json!({"shelf": "a-1"});
        let mut unknown_member = draft.clone();
        unknown_member["extra"] = true.into();
        let mut wrong_partition = draft.clone();
        wrong_partition["partition"] = serde_json::json!({"kind": "tenant", "tenant": "other"});
        let mut denied_but_accepted = draft;
        denied_but_accepted["outcome"] = "denied".into();
        for invalid in [
            kindless,
            unknown_member,
            wrong_partition,
            denied_but_accepted,
        ] {
            assert!(AuditStoredRecord::admit(invalid, Utc::now()).is_err());
        }
    }
}
