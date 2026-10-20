use serde::{Serialize, de::DeserializeOwned};
use veoveo_types::{BuildId, DeploymentId, EnvironmentId};

fn canonical_id<I: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(id: I) {
    let encoded = serde_json::to_value(&id).unwrap();
    let text = encoded.as_str().unwrap().to_owned();
    let uuid = uuid::Uuid::parse_str(&text).unwrap();
    assert_eq!(uuid.get_version_num(), 7);
    assert_eq!(uuid.to_string(), text);
    assert_eq!(serde_json::from_value::<I>(encoded).unwrap(), id);

    for invalid in [
        "00000000-0000-0000-0000-000000000000".to_owned(),
        "550e8400-e29b-41d4-a716-446655440000".to_owned(),
        "019a05cb-6c00-7000-0712-3456789abcde".to_owned(),
        "019A05CB-6C00-7000-8712-3456789ABCDE".to_owned(),
        text.replace('-', ""),
        format!("urn:uuid:{text}"),
    ] {
        assert!(serde_json::from_value::<I>(serde_json::json!(invalid)).is_err());
    }
}

#[test]
fn execution_ids_admit_only_canonical_uuid_v7_across_serde() {
    canonical_id(EnvironmentId::new());
    canonical_id(DeploymentId::new());
    canonical_id(BuildId::new());
}

#[test]
fn execution_id_schemas_declare_version_variant_and_spelling() {
    for schema in [
        schemars::schema_for!(EnvironmentId),
        schemars::schema_for!(DeploymentId),
        schemars::schema_for!(BuildId),
    ] {
        let schema = serde_json::to_value(schema).unwrap();
        assert_eq!(schema["format"], "uuid");
        assert_eq!(
            schema["pattern"],
            "^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"
        );
    }
}

#[test]
fn builds_and_deployments_allocate_independent_incarnations() {
    let first_build = BuildId::new();
    let rebuilt = BuildId::new();
    let first_deployment = DeploymentId::new();
    let replacement = DeploymentId::new();
    assert_ne!(first_build, rebuilt);
    assert_ne!(first_deployment, replacement);
}
