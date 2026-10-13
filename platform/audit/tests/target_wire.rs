//! Owner target registration and detail declaration preserve the signed record object.
mod support;

use chrono::Utc;
use support::audit_owner::{
    ItemAuditTarget, ItemId, Note, declare_audit_detail, register_audit_target,
};
use veoveo_audit::{
    integrity::{canonical_bytes, record_root},
    *,
};

fn registry() -> AuditRegistry {
    let mut builder = AuditRegistry::builder();
    register_audit_target(&mut builder).unwrap();
    declare_audit_detail(&mut builder).unwrap();
    builder.build()
}

#[test]
fn owner_target_and_detail_keep_canonical_record_bytes_and_merkle_leaf() {
    let registry = registry();
    let item = ItemId::parse("019f25e8-cf39-7000-8000-000000000001").unwrap();
    let target = registry.target(ItemAuditTarget { item }).unwrap();
    let record = AuditRecord {
        draft: AuditDraft::builder(
            AuditRequest::background(),
            target,
            registry.detail(&Note { count: 7 }).unwrap(),
            AuditOutcome::Succeeded,
            AuditReason::Accepted,
        )
        .build()
        .unwrap(),
        recorded_at: Utc::now(),
    };
    let mut wire = serde_json::to_value(&record).unwrap();
    wire["draft"]["target"] =
        serde_json::json!({"kind":"item","item":"019f25e8-cf39-7000-8000-000000000001"});
    wire["draft"]["class"] = "fixture.note".into();
    wire["draft"]["detail"] = serde_json::json!({"count": 7});
    assert_eq!(
        canonical_bytes(&record).unwrap(),
        canonical_bytes(&wire).unwrap()
    );
    let decoded: AuditRecord = registry.decoder().from_value(wire).unwrap();
    assert_eq!(record, decoded);
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
        "42e8e4072628a28d5b1931aff66dd93454f2e49b1de75dfe0e4dc4567a7adf7d"
    );
}
