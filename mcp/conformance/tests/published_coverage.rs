//! The published requirement coverage map agrees with the conformance suite.
use serde_json::Value;
use std::path::Path;

#[test]
fn published_coverage_matches_the_suite() {
    let actual =
        serde_json::to_value(veoveo_mcp_conformance::requirements::requirement_coverage()).unwrap();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("catalog/coverage.json");
    if std::env::var_os("VEOVEO_CAPTURE_CATALOG").is_some() {
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&actual).unwrap()),
        )
        .unwrap();
        return;
    }
    let expected: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(actual, expected, "catalog/coverage.json is stale");
}
