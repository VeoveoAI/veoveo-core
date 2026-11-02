#[path = "support/naming.rs"]
mod naming_baseline;
use schemars::schema_for;
use serde_json::{Value, json};
use veoveo_types::{AccessLevel, InvocationAuthority};

#[test]
fn authority_schemas_match_the_shared_wire_contract() {
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/authority_schemas.json")).unwrap();
    let actual = json!({
        "AccessLevel": schema_for!(AccessLevel),
        "InvocationAuthority": schema_for!(InvocationAuthority),
    });
    assert_eq!(naming_baseline::constraints(actual), expected);
}

fn authority() -> Value {
    json!({
        "tenant": "tenant", "policy_revision": "r1",
        "provenance": {"mode":"delegated","initiator":"operator","delegation_id":"mission-grant"}
    })
}

#[test]
fn authority_retains_attribution_and_typed_nested_identity() {
    let wire = authority();
    let value: InvocationAuthority = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(value.tenant.as_str(), "tenant");
    assert_eq!(serde_json::to_value(&value).unwrap(), wire);
    // View composition identities hash these bytes, including struct field order.
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"tenant":"tenant","policy_revision":"r1","provenance":{"mode":"delegated","initiator":"operator","delegation_id":"mission-grant"}}"#
    );
    for (path, replacement) in [
        ("/tenant", json!("")),
        ("/policy_revision", json!("two revisions")),
        ("/provenance/delegation_id", Value::Null),
    ] {
        let mut invalid = authority();
        *invalid.pointer_mut(path).unwrap() = replacement;
        assert!(
            serde_json::from_value::<InvocationAuthority>(invalid).is_err(),
            "accepted {path}"
        );
    }
    let mut work_context = authority();
    work_context["work_context"] = json!("operations");
    assert!(
        serde_json::from_value::<InvocationAuthority>(work_context).is_err(),
        "an authority names no Work Context"
    );
}

#[test]
fn capability_levels_are_ordered() {
    use AccessLevel::{Admin, Read, Write};
    for (held, allowed) in [
        (Read, [true, false, false]),
        (Write, [true, true, false]),
        (Admin, [true, true, true]),
    ] {
        for (required, expected) in [Read, Write, Admin].into_iter().zip(allowed) {
            assert_eq!(held.allows(required), expected);
        }
    }
}
