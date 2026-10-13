//! An owner's class, reason, target and acting server keep the signed record object.
mod support;

use chrono::Utc;
use support::audit_owner::{ItemId, Note, declare_audit_detail, item_target};
use veoveo_audit::{
    integrity::{canonical_bytes, record_root},
    *,
};

fn registry() -> AuditRegistry {
    let mut builder = AuditRegistry::builder();
    declare_audit_detail(&mut builder).unwrap();
    builder.build()
}

#[test]
fn owner_records_keep_canonical_record_bytes_and_merkle_leaf() {
    let registry = registry();
    let item = ItemId::parse("019f25e8-cf39-7000-8000-000000000001").unwrap();
    let record = AuditRecord {
        draft: AuditDraft::builder(
            AuditRequest::background(),
            registry.detail(&Note { count: 7 }).unwrap(),
            AuditOutcome::Denied,
        )
        .target(item_target(item))
        .reason(AuditReason::parse("shelf_full").unwrap())
        .via(AuditVia {
            server: "media".parse().unwrap(),
            task: None,
        })
        .build()
        .unwrap(),
        recorded_at: Utc::now(),
    };
    let wire = serde_json::to_value(&record).unwrap();
    assert_eq!(
        wire["draft"]["target"],
        "fixture://items/019f25e8-cf39-7000-8000-000000000001"
    );
    assert_eq!(wire["draft"]["reason"], "shelf_full");
    let decoded: AuditRecord = registry.decoder().from_value(wire.clone()).unwrap();
    assert_eq!(record, decoded);
    assert_eq!(
        canonical_bytes(&record).unwrap(),
        canonical_bytes(&wire).unwrap()
    );
    assert_eq!(
        registry
            .read::<Note>(decoded.draft.class(), decoded.draft.detail())
            .unwrap(),
        Note { count: 7 }
    );
    assert_eq!(
        record_root(&[record]).unwrap(),
        record_root(&[decoded]).unwrap()
    );
}

#[test]
fn literal_owner_record_hash() {
    let record: AuditRecord = registry()
        .decoder()
        .from_str(include_str!("../testdata/owner-record.json"))
        .unwrap();
    assert_eq!(
        record_root(&[record]).unwrap().hex(),
        "40ac40b8d8106cd681d4c408c44c9d2f8e2b737857f247436d59d4939fa0e700"
    );
}
