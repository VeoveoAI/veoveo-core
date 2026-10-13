//! OCSF 1.9.0 mapping of the record envelope. A record with an actor and a source
//! address is an API Activity whose operation is the record's class; any other record
//! is a Base Event. The outcome maps to `status_id` and the Security Control profile's
//! `action_id`, the owner's reason to `status_code`, the acting server to
//! `actor.application` and the target to `resources`. Unavailable facts stay
//! unavailable, and `unmapped.veoveo` preserves the complete signed source record with
//! its owner's detail.
use serde::Serialize;
use std::net::IpAddr;
use veoveo_audit_contract::*;
use veoveo_types::{PrincipalId, TenantId};

#[derive(Serialize)]
pub struct Event<'a> {
    category_uid: u16,
    class_uid: u16,
    activity_id: u8,
    activity_name: &'a str,
    type_uid: u32,
    severity_id: u8,
    status_id: u8,
    status: &'static str,
    action_id: u8,
    action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    status_code: Option<&'a AuditReason>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    resources: Vec<Resource<'a>>,
    profiles: [&'static str; 1],
    time: i64,
    metadata: Metadata<'a>,
    #[serde(flatten)]
    fields: Fields<'a>,
    unmapped: Source<'a>,
}
#[derive(Serialize)]
struct Metadata<'a> {
    version: &'static str,
    product: Product,
    uid: AuditRecordId,
    original_event_uid: AuditRecordId,
    correlation_uid: AuditRequestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    tenant_uid: Option<&'a TenantId>,
    processed_time: i64,
}
#[derive(Serialize)]
struct Product {
    name: &'static str,
    vendor_name: &'static str,
}
#[derive(Serialize)]
struct Source<'a> {
    veoveo: &'a AuditStoredRecord,
}
#[derive(Serialize)]
#[serde(untagged)]
enum Fields<'a> {
    Api {
        actor: Actor<'a>,
        api: Api<'a>,
        src_endpoint: Endpoint,
    },
    Base {},
}
#[derive(Serialize)]
struct Actor<'a> {
    user: User<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    application: Option<Application<'a>>,
}
/// The server that acted for the user, and the Task the call came from.
#[derive(Serialize)]
struct Application<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    uid: Option<String>,
}
#[derive(Serialize)]
struct Resource<'a> {
    uid: &'a str,
}
#[derive(Serialize)]
struct User<'a> {
    uid: &'a PrincipalId,
    type_id: u8,
}
#[derive(Serialize)]
struct Api<'a> {
    operation: &'a str,
}
#[derive(Serialize)]
struct Endpoint {
    ip: IpAddr,
}
impl<'a> User<'a> {
    fn actor(actor: &'a AuditActor) -> Self {
        Self {
            uid: &actor.principal,
            type_id: match actor.kind {
                AuditPrincipalKind::User => 1,
                AuditPrincipalKind::Service => 4,
                AuditPrincipalKind::Capability => 0,
            },
        }
    }
}
impl<'a> Event<'a> {
    /// Map a record as stored. The mapping reads only the envelope, so it needs no
    /// owner's target codec or detail declaration.
    pub fn from_record(record: &'a AuditStoredRecord) -> Self {
        let draft = record;
        let (class_uid, activity_id, activity_name, fields) = classify(record);
        let (status_id, status, action_id, action) = match draft.outcome() {
            AuditOutcome::Success => (1, "Success", 1, "Allowed"),
            AuditOutcome::Failure => (2, "Failure", 1, "Allowed"),
            AuditOutcome::Denied => (2, "Failure", 2, "Denied"),
            AuditOutcome::Unknown => (0, "Unknown", 0, "Unknown"),
        };
        Self {
            category_uid: class_uid / 1000,
            class_uid,
            activity_id,
            activity_name,
            type_uid: u32::from(class_uid) * 100 + u32::from(activity_id),
            severity_id: 1,
            status_id,
            status,
            action_id,
            action,
            status_code: draft.reason(),
            resources: draft
                .target()
                .map(|target| Resource {
                    uid: target.as_str(),
                })
                .into_iter()
                .collect(),
            profiles: ["security_control"],
            time: draft.occurred_at().timestamp_millis(),
            metadata: Metadata {
                version: "1.9.0",
                product: Product {
                    name: "Veoveo",
                    vendor_name: "Bioma",
                },
                uid: draft.id(),
                original_event_uid: draft.id(),
                correlation_uid: draft.request().id,
                tenant_uid: draft.actor().and_then(|actor| actor.tenant.as_ref()),
                processed_time: record.recorded_at().timestamp_millis(),
            },
            fields,
            unmapped: Source { veoveo: record },
        }
    }
}
fn classify(record: &AuditStoredRecord) -> (u16, u8, &str, Fields<'_>) {
    let draft = record;
    let class = draft.class().as_str();
    if let (Some(actor), Some(ip)) = (draft.actor(), draft.request().source_ip) {
        return (
            6003,
            99,
            class,
            Fields::Api {
                actor: Actor {
                    user: User::actor(actor),
                    application: draft.via().map(|via| Application {
                        name: via.server.as_str(),
                        uid: via.task.map(|task| task.as_uuid().to_string()),
                    }),
                },
                src_endpoint: Endpoint { ip },
                api: Api { operation: class },
            },
        );
    }
    (0, 99, class, Fields::Base {})
}
