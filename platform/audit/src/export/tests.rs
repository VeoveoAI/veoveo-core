use super::*;
use chrono::Utc;

/// A record as stored, the form export reads.
pub(super) fn record(
    detail: AuditDetail,
    target: AuditTarget,
    identified: bool,
) -> AuditStoredRecord {
    let mut request = AuditRequest::background();
    if identified {
        request.source_ip = Some("192.0.2.1".parse().unwrap());
    }
    let mut builder = AuditDraft::builder(
        request,
        target,
        detail,
        AuditOutcome::Allowed,
        AuditReason::Accepted,
    );
    if identified {
        builder = builder.actor(AuditActor {
            principal: "operator".parse().unwrap(),
            kind: AuditPrincipalKind::User,
            tenant: None,
            oauth_client: None,
            delegating_principal: None,
        });
    }
    AuditRecord {
        draft: builder.build().unwrap(),
        recorded_at: Utc::now(),
    }
    .stored()
}
#[test]
fn ocsf_maps_identified_records_to_api_activity_named_by_class() {
    let context = AuditTarget::WorkContext {
        tenant: "tenant".parse().unwrap(),
        context: "operations".parse().unwrap(),
    };
    let anonymous = record(
        veoveo_audit_contract::AuditRegistry::platform()
            .detail(&veoveo_audit_contract::platform::TaskCancel {})
            .unwrap(),
        context.clone(),
        false,
    );
    let wire = serde_json::to_value(ocsf::Event::from_record(&anonymous)).unwrap();
    assert_eq!(wire["class_uid"], 0);
    assert!(wire.get("actor").is_none());
    assert!(wire.get("src_endpoint").is_none());
    assert_eq!(
        wire["unmapped"]["veoveo"],
        serde_json::to_value(&anonymous).unwrap()
    );
    let identified = record(
        veoveo_audit_contract::AuditRegistry::platform()
            .detail(&veoveo_audit_contract::platform::TaskCancel {})
            .unwrap(),
        context,
        true,
    );
    let wire = serde_json::to_value(ocsf::Event::from_record(&identified)).unwrap();
    assert_eq!(wire["class_uid"], 6003);
    assert_eq!(wire["type_uid"], 600399);
    assert_eq!(wire["activity_name"], "task.cancel");
    assert_eq!(wire["api"]["operation"], "task.cancel");
    assert_eq!(wire["actor"]["user"]["uid"], "operator");
    assert_eq!(wire["status"], "Allowed");
}
#[test]
fn destination_identity_covers_storage_and_retention_but_never_credentials() {
    let cfg: S3Config = serde_json::from_str(r#"{"endpoint":"https://store.example.test","region":"us-east-1","bucket":"audit-fixture","prefix":"audit","objectLock":{"mode":"disabled"}}"#).unwrap();
    let first = config::destination_id("s3", &cfg).unwrap();
    let mut changed = cfg.clone();
    changed.prefix = "another-installation".to_owned().try_into().unwrap();
    assert_ne!(first, config::destination_id("s3", &changed).unwrap());
    changed = cfg.clone();
    changed.object_lock = ObjectLock::Compliance {
        days: std::num::NonZeroU32::new(365).unwrap(),
    };
    assert_ne!(first, config::destination_id("s3", &changed).unwrap());
    assert!(
        "a/../b"
            .to_owned()
            .try_into()
            .map(|_: ObjectPrefix| ())
            .is_err()
    );
    assert!(config::endpoint(&"https://user:secret@example.test".parse().unwrap(), false).is_err());
}
#[test]
fn export_retry_bytes_and_otlp_correlation_are_stable() {
    let record = record(
        veoveo_audit_contract::AuditRegistry::platform()
            .detail(&veoveo_audit_contract::platform::AuditView {})
            .unwrap(),
        AuditTarget::AuditLog {
            partition: AuditPartition::Installation,
        },
        true,
    );
    let records = vec![record];
    let block = crate::integrity::AuditSigningKey::from_seed(&[7; 32])
        .seal(AuditPartition::Installation, None, &records, Utc::now())
        .unwrap();
    let first = Bundle::ocsf(&block, &records).unwrap();
    assert_eq!(
        first.hashes(),
        Bundle::ocsf(&block, &records).unwrap().hashes()
    );
    assert!(first.content.ends_with(b"\n"));
    let bytes = otlp::OtlpDestination::payload(&records).unwrap();
    assert_eq!(bytes, otlp::OtlpDestination::payload(&records).unwrap());
    let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let log = &wire["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0];
    assert_eq!(log["traceId"], records[0].request().trace_id.as_str());
    assert_eq!(log["spanId"], records[0].request().span_id.as_str());
    assert_eq!(log["eventName"], "veoveo.audit.audit.view");
    assert!(log["timeUnixNano"].is_string());
    assert!(encode(&vec!["oversized"; 100], 4).is_err());
}
