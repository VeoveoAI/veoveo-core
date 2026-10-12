//! The catalog files published with the contract agree with the types that define them.
use serde_json::Value;
use std::path::Path;
use veoveo_mcp_contract::docs::{ComplianceProfile, catalog::requirement_catalog};

/// Compares a published file with its generated value; `VEOVEO_CAPTURE_CATALOG` rewrites it.
fn published(path: &str, actual: Value) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    if std::env::var_os("VEOVEO_CAPTURE_CATALOG").is_some() {
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&actual).unwrap()),
        )
        .unwrap();
        return;
    }
    let expected: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(actual, expected, "{} is stale", path.display());
}

#[test]
fn published_requirement_catalog_matches_the_contract() {
    published(
        "catalog/requirements.json",
        serde_json::to_value(requirement_catalog()).unwrap(),
    );
}

#[test]
fn published_compliance_schema_matches_the_contract() {
    let schema = serde_json::to_value(schemars::schema_for!(ComplianceProfile)).unwrap();
    assert_eq!(
        schema["$id"],
        "https://veoveo.ai/formats/hosted-mcp-compliance"
    );
    published("catalog/compliance-profile.schema.json", schema);
}
