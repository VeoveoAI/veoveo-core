//! Admission, wire form and the published schema of the component lock.
use std::path::Path;

use serde_json::{Value, json};
use veoveo_installation_contract::{
    COMPONENT_LOCK_FORMAT, ComponentLock, ComponentLockError, ComponentName, ComponentRepository,
    GitRevision, LockedComponent,
};

const EXAMPLE: &str = include_str!("../testdata/component-lock.json");

fn example() -> Value {
    serde_json::from_str(EXAMPLE).unwrap()
}

fn decode(value: Value) -> Result<ComponentLock, serde_json::Error> {
    serde_json::from_value(value)
}

/// The example with one member of its first component replaced.
fn with_component(member: &str, value: Value) -> Value {
    let mut lock = example();
    lock["components"][0][member] = value;
    lock
}

fn schema() -> Value {
    serde_json::to_value(schemars::schema_for!(ComponentLock)).unwrap()
}

#[test]
fn the_example_lock_decodes_and_keeps_its_wire_form() {
    let lock = decode(example()).unwrap();
    assert_eq!(serde_json::to_value(&lock).unwrap(), example());
    let hub = ComponentName::parse("veoveo-hub").unwrap();
    let pin = lock.component(&hub).unwrap();
    assert_eq!(pin.repository().owner(), "VeoveoAI");
    assert_eq!(pin.repository().name(), hub);
    assert_eq!(
        pin.revision().as_str(),
        "0123456789abcdef0123456789abcdef01234567"
    );
    let names: Vec<_> = lock
        .components()
        .iter()
        .map(|component| component.name().as_str())
        .collect();
    assert_eq!(names, ["veoveo-hub", "veoveo-mcp-time"]);
    assert!(
        lock.component(&ComponentName::parse("veoveo-mcp-map").unwrap())
            .is_none()
    );
}

#[test]
fn constructors_apply_the_same_admission_as_decoding() {
    let name = ComponentName::parse("veoveo-mcp-time").unwrap();
    let revision = GitRevision::parse("89abcdef0123456789abcdef0123456789abcdef").unwrap();
    let time = LockedComponent::new(
        name.clone(),
        ComponentRepository::parse("https://github.com/acme/veoveo-mcp-time").unwrap(),
        revision.clone(),
    )
    .unwrap();
    let lock = ComponentLock::new(vec![time.clone()]).unwrap();
    assert_eq!(
        serde_json::to_value(&lock).unwrap(),
        json!({
            "format": COMPONENT_LOCK_FORMAT,
            "components": [{
                "name": "veoveo-mcp-time",
                "repository": "https://github.com/acme/veoveo-mcp-time",
                "revision": "89abcdef0123456789abcdef0123456789abcdef"
            }]
        })
    );
    assert!(matches!(
        LockedComponent::new(
            name,
            ComponentRepository::parse("https://github.com/acme/veoveo-mcp-map").unwrap(),
            revision,
        ),
        Err(ComponentLockError::RepositoryName { .. })
    ));
    assert!(matches!(
        ComponentLock::new(vec![time.clone(), time]),
        Err(ComponentLockError::DuplicateComponent(_))
    ));
    assert!(matches!(
        ComponentLock::new(Vec::new()),
        Err(ComponentLockError::Empty)
    ));
}

#[test]
fn names_follow_the_repository_naming_rule() {
    for name in [
        "veoveo-hub",
        "veoveo-hub-sdk",
        "veoveo-mcp-time",
        "veoveo-runtime-embedding",
        "veoveo-app-console",
        "veoveo-showcase-uav",
        "veoveo-mcp-time-zones2",
    ] {
        assert!(ComponentName::parse(name).is_ok(), "{name}");
    }
    let longest = format!("veoveo-mcp-{}", "a".repeat(89));
    assert_eq!(longest.len(), 100);
    assert!(ComponentName::parse(&longest).is_ok());
    for name in [
        "veoveo-".to_owned(),
        "veoveo".to_owned(),
        "veoveo-mcp-".to_owned(),
        "veoveo-mcp-time-".to_owned(),
        "veoveo-mcp--time".to_owned(),
        "veoveo-1time".to_owned(),
        "veoveo-mcp-Time".to_owned(),
        "veoveo-mcp-time_zones".to_owned(),
        "vv-mcp-time".to_owned(),
        "mcp-time".to_owned(),
        format!("veoveo-mcp-{}", "a".repeat(90)),
    ] {
        assert!(
            matches!(
                ComponentName::parse(&name),
                Err(ComponentLockError::Name(_))
            ),
            "{name}"
        );
        assert!(
            decode(with_component("name", json!(name))).is_err(),
            "{name}"
        );
    }
}

#[test]
fn repositories_are_github_urls_named_after_their_component() {
    let repository =
        ComponentRepository::parse("https://github.com/Acme-Robotics/veoveo-mcp-inventory")
            .unwrap();
    assert_eq!(repository.owner(), "Acme-Robotics");
    assert_eq!(repository.name().as_str(), "veoveo-mcp-inventory");
    for url in [
        "http://github.com/VeoveoAI/veoveo-hub",
        "https://gitlab.com/VeoveoAI/veoveo-hub",
        "https://www.github.com/VeoveoAI/veoveo-hub",
        "https://github.com:443/VeoveoAI/veoveo-hub",
        "https://GitHub.com/VeoveoAI/veoveo-hub",
        "https://github.com/VeoveoAI/veoveo-hub/",
        "https://github.com/VeoveoAI/veoveo-hub.git",
        "https://github.com/VeoveoAI/veoveo-hub/tree/main",
        "https://github.com/VeoveoAI/veoveo-hub?tab=readme",
        "https://github.com/VeoveoAI/veoveo-hub#readme",
        "https://user@github.com/VeoveoAI/veoveo-hub",
        "https://github.com/-VeoveoAI/veoveo-hub",
        "https://github.com/VeoveoAI-/veoveo-hub",
        "https://github.com/Veoveo--AI/veoveo-hub",
        "https://github.com/Veoveo_AI/veoveo-hub",
        "https://github.com//veoveo-hub",
        "github.com/VeoveoAI/veoveo-hub",
    ] {
        assert!(
            matches!(
                ComponentRepository::parse(url),
                Err(ComponentLockError::Repository(_))
            ),
            "{url}"
        );
        assert!(
            decode(with_component("repository", json!(url))).is_err(),
            "{url}"
        );
    }
    let long_owner = format!("https://github.com/{}/veoveo-mcp-time", "a".repeat(40));
    assert!(ComponentRepository::parse(&long_owner).is_err());
    let longest_owner = format!("https://github.com/{}/veoveo-mcp-time", "a".repeat(39));
    assert!(ComponentRepository::parse(&longest_owner).is_ok());
    // The repository names the component it holds.
    assert!(
        decode(with_component(
            "repository",
            json!("https://github.com/VeoveoAI/veoveo-mcp-time")
        ))
        .is_err()
    );
}

#[test]
fn revisions_are_full_lowercase_commit_ids() {
    assert!(GitRevision::parse("0123456789abcdef0123456789abcdef01234567").is_ok());
    for revision in [
        "0123456".to_owned(),
        "0123456789ABCDEF0123456789ABCDEF01234567".to_owned(),
        "0123456789abcdef0123456789abcdef0123456".to_owned(),
        "0123456789abcdef0123456789abcdef012345678".to_owned(),
        "g123456789abcdef0123456789abcdef01234567".to_owned(),
        "a".repeat(64),
        "0".repeat(40),
        "main".to_owned(),
        "v0.1.0".to_owned(),
    ] {
        assert!(
            matches!(
                GitRevision::parse(&revision),
                Err(ComponentLockError::Revision(_))
            ),
            "{revision}"
        );
        assert!(
            decode(with_component("revision", json!(revision))).is_err(),
            "{revision}"
        );
    }
}

#[test]
fn documents_are_closed_objects_of_their_format() {
    let mut other_format = example();
    other_format["format"] = json!("https://veoveo.ai/formats/deployment-lock");
    let mut unversioned = example();
    unversioned["format"] = json!("https://veoveo.ai/formats/component-lock/v1");
    let mut no_format = example();
    no_format.as_object_mut().unwrap().remove("format");
    let mut extra = example();
    extra["installation"] = json!("bioma");
    let mut extra_member = example();
    extra_member["components"][0]["chart"] = json!("1.0.0");
    let mut empty = example();
    empty["components"] = json!([]);
    let mut duplicate = example();
    let first = duplicate["components"][0].clone();
    duplicate["components"].as_array_mut().unwrap().push(first);
    let mut missing = example();
    missing["components"][0]
        .as_object_mut()
        .unwrap()
        .remove("revision");
    for (case, document) in [
        ("another format", other_format),
        ("a versioned format", unversioned),
        ("no format", no_format),
        ("an unknown member", extra),
        ("an unknown component member", extra_member),
        ("no components", empty),
        ("a duplicate component", duplicate),
        ("a missing revision", missing),
    ] {
        assert!(decode(document).is_err(), "{case}");
    }
    assert!(matches!(
        decode(json!({"format": COMPONENT_LOCK_FORMAT, "components": []}))
            .unwrap_err()
            .to_string()
            .as_str(),
        message if message.contains("at least one component")
    ));
}

#[test]
fn the_schema_is_identified_by_the_format_and_closes_every_object() {
    let schema = schema();
    assert_eq!(schema["$id"], COMPONENT_LOCK_FORMAT);
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["$defs"]["LockedComponent"]["additionalProperties"],
        false
    );
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert!(validator.is_valid(&example()));
    let mut other_format = example();
    other_format["format"] = json!("https://veoveo.ai/formats/other");
    let mut empty = example();
    empty["components"] = json!([]);
    let mut extra = example();
    extra["components"][0]["chart"] = json!("1.0.0");
    for (case, document) in [
        ("another format", other_format),
        ("no components", empty),
        ("an unknown component member", extra),
        (
            "an invalid repository name",
            with_component("name", json!("veoveo-invalid_name")),
        ),
        (
            "a short revision",
            with_component("revision", json!("0123456")),
        ),
        (
            "a zero revision",
            with_component("revision", json!("0".repeat(40))),
        ),
        (
            "a repository on another host",
            with_component(
                "repository",
                json!("https://gitlab.com/VeoveoAI/veoveo-hub"),
            ),
        ),
    ] {
        assert!(!validator.is_valid(&document), "{case}");
    }
}

#[test]
fn the_published_schema_matches_the_types() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas/component-lock.schema.json");
    let actual = schema();
    if std::env::var_os("VEOVEO_CAPTURE_CATALOG").is_some() {
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&actual).unwrap()),
        )
        .unwrap();
        return;
    }
    let published: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(actual, published, "{} is stale", path.display());
}
