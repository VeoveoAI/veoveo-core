//! Computer owner registration preserves the frozen signed record object.
use chrono::Utc;
use veoveo_audit::{
    integrity::{canonical_bytes, record_root},
    *,
};
#[path = "../../../testing/fixtures/computer_target.rs"]
mod computer_target;
use computer_target::{ComputerAuditTarget, ComputerId, register_audit_target};
#[test]
fn owner_target_keeps_canonical_record_bytes_and_merkle_leaf() {
    let mut builder = AuditTargetRegistry::builder();
    register_audit_target(&mut builder).unwrap();
    let registry = builder.build();
    let computer = ComputerId::parse("019f25e8-cf39-7000-8000-000000000001").unwrap();
    let target = registry.target(ComputerAuditTarget { computer }).unwrap();
    let record = AuditRecord {
        draft: AuditDraft::builder(
            AuditRequest::background(),
            target,
            AuditDetail::Computer {
                activity: ComputerActivity::Create,
                stage: ComputerAuditStage::Reserved,
                task: None,
            },
            AuditOutcome::Succeeded,
            AuditReason::Accepted,
        )
        .build()
        .unwrap(),
        recorded_at: Utc::now(),
    };
    let mut frozen = serde_json::to_value(&record).unwrap();
    frozen["draft"]["target"] =
        serde_json::json!({"kind":"computer","computer":"019f25e8-cf39-7000-8000-000000000001"});
    assert_eq!(
        canonical_bytes(&record).unwrap(),
        canonical_bytes(&frozen).unwrap()
    );
    let decoded: AuditRecord = registry.decoder().from_value(frozen).unwrap();
    assert_eq!(record, decoded);
    assert_eq!(
        record_root(&[record]).unwrap(),
        record_root(&[decoded]).unwrap()
    );
}

#[test]
fn literal_frozen_computer_record_hash() {
    let mut builder = AuditTargetRegistry::builder();
    register_audit_target(&mut builder).unwrap();
    let registry = builder.build();
    let record: AuditRecord = registry
        .decoder()
        .from_str(include_str!("../testdata/computer-record.json"))
        .unwrap();
    assert_eq!(
        record_root(&[record]).unwrap().hex(),
        "5889060b4647cf968f69ed90839480a012b33060a88499fe7d8fda03f3bcf485"
    );
}
