//! OCSF 1.9.0 mapping of the record envelope. A record with an actor and a source
//! address is an API Activity whose operation is the record's class; any other record
//! is a Base Event. Unavailable facts stay unavailable, and `unmapped.veoveo` preserves
//! the complete signed source record with its owner's detail.
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
    veoveo: &'a AuditRecord,
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
    pub fn from_record(record: &'a AuditRecord) -> Self {
        let draft = &record.draft;
        let (class_uid, activity_id, activity_name, fields) = classify(record);
        let (status_id, status) = match draft.outcome() {
            AuditOutcome::Succeeded => (1, "Success"),
            AuditOutcome::Failed | AuditOutcome::Denied => (2, "Failure"),
            // Admission does not assert that an operation has completed.
            AuditOutcome::Allowed => (99, "Allowed"),
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
                processed_time: record.recorded_at.timestamp_millis(),
            },
            fields,
            unmapped: Source { veoveo: record },
        }
    }
}
fn classify(record: &AuditRecord) -> (u16, u8, &str, Fields<'_>) {
    let draft = &record.draft;
    let class = draft.class().as_str();
    if let (Some(actor), Some(ip)) = (draft.actor(), draft.request().source_ip) {
        return (
            6003,
            99,
            class,
            Fields::Api {
                actor: Actor {
                    user: User::actor(actor),
                },
                src_endpoint: Endpoint { ip },
                api: Api { operation: class },
            },
        );
    }
    (0, 99, class, Fields::Base {})
}
