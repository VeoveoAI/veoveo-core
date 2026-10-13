//! Independently implemented owner codecs qualify the public extension interface.
use veoveo_audit_contract::*;
#[derive(Debug, Clone, PartialEq, Eq, schemars::JsonSchema)]
#[schemars(deny_unknown_fields, transform = widget_schema)]
struct Widget {
    id: uuid::Uuid,
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WidgetWire {
    Widget { id: uuid::Uuid },
}
impl serde::Serialize for Widget {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(&WidgetWire::Widget { id: self.id }, s)
    }
}
impl<'de> serde::Deserialize<'de> for Widget {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let WidgetWire::Widget { id } = <WidgetWire as serde::Deserialize>::deserialize(d)?;
        Ok(Self { id })
    }
}
impl AuditTargetOwner for Widget {
    const KIND: &'static str = "widget";
}
fn registry() -> (AuditRegistry, AuditTargetRegistration<Widget>) {
    let mut builder = AuditRegistry::builder();
    let key = builder.register_target::<Widget>().unwrap_or_else(|error| {
        panic!(
            "{error}: {}",
            serde_json::to_value(schemars::schema_for!(Widget)).unwrap()
        )
    });
    (builder.build(), key)
}
#[test]
fn owner_admission_preserves_wire_and_retrieves_typed_payload() {
    let (registry, key) = registry();
    let widget = Widget {
        id: uuid::Uuid::new_v4(),
    };
    let target = key.target(&registry, widget.clone()).unwrap();
    let json = serde_json::to_value(&target).unwrap();
    assert_eq!(json, serde_json::to_value(&widget).unwrap());
    assert_eq!(key.get(&registry, &target).unwrap(), &widget);
    let decoded: AuditTarget = registry.decoder().from_value(json).unwrap();
    assert_eq!(decoded, target);
    let (other, other_key) = self::registry();
    assert!(matches!(
        other.validate(&target),
        Err(AuditTargetError::Registry)
    ));
    assert!(key.get(&other, &target).is_err());
    assert!(other_key.get(&registry, &target).is_err());
    assert_eq!(
        registry
            .clone()
            .decoder()
            .from_value::<AuditTarget>(serde_json::to_value(&target).unwrap())
            .unwrap(),
        target
    );
}
#[test]
fn unknown_malformed_duplicate_and_unbound_targets_fail() {
    let (registry, _) = registry();
    for input in [
        r#"{"kind":"widget","id":"bad"}"#,
        r#"{"kind":"widget","id":"00000000-0000-0000-0000-000000000000","extra":1}"#,
        r#"{"kind":"missing"}"#,
        r#"{"kind":"installation","kind":"widget"}"#,
        r#"{"kind":"installation","extra":1}"#,
    ] {
        assert!(
            registry.decoder().from_str::<AuditTarget>(input).is_err(),
            "{input}"
        );
    }
    assert!(matches!(
        AuditRegistry::platform().decoder().from_str::<AuditTarget>(
            r#"{"kind":"widget","id":"00000000-0000-0000-0000-000000000000"}"#
        ),
        Err(AuditDecodeError::Target(AuditTargetError::Unbound(_)))
    ));
    let mut builder = AuditRegistry::builder();
    builder.register_target::<Widget>().unwrap_or_else(|error| {
        panic!(
            "{error}: {}",
            serde_json::to_value(schemars::schema_for!(Widget)).unwrap()
        )
    });
    assert!(builder.register_target::<Widget>().is_err());
}
#[test]
fn reader_schema_is_composition_closed() {
    let empty = serde_json::to_value(reader_schema(&AuditRegistry::platform()).unwrap()).unwrap();
    assert!(empty.pointer("/$defs/ComputerId").is_none());
    assert!(
        !empty
            .pointer("/$defs/AuditTarget")
            .unwrap()
            .to_string()
            .contains("computer")
    );
    assert!(!empty.to_string().contains("widget"));
    let (registry, _) = registry();
    let schema = serde_json::to_value(reader_schema(&registry).unwrap()).unwrap();
    assert!(
        schema
            .pointer("/$defs/AuditTarget/oneOf")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .any(|branch| branch.pointer("/properties/kind/const")
                == Some(&serde_json::json!("widget")))
    );
}
#[test]
fn core_draft_contextual_decode_keeps_validation() {
    let registry = AuditRegistry::platform();
    let draft = AuditDraft::builder(
        AuditRequest::background(),
        AuditTarget::Installation,
        registry.detail(&platform::AuditView {}).unwrap(),
        AuditOutcome::Succeeded,
        AuditReason::Accepted,
    )
    .build()
    .unwrap();
    let json = serde_json::to_value(&draft).unwrap();
    assert_eq!(
        registry
            .decoder()
            .from_value::<AuditDraft>(json.clone())
            .unwrap(),
        draft
    );
    let mut invalid = json.clone();
    invalid["partition"] = serde_json::json!({"kind":"tenant","tenant":"wrong"});
    assert!(
        registry
            .decoder()
            .from_value::<AuditDraft>(invalid)
            .is_err()
    );
    let mut undeclared = json;
    undeclared["class"] = "fixture.unknown".into();
    assert!(matches!(
        registry.decoder().from_value::<AuditDraft>(undeclared),
        Err(AuditDecodeError::Detail(AuditDetailError::Undeclared(_)))
    ));
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Note {
    count: u32,
}
impl AuditDetailType for Note {
    const CLASS: &'static str = "fixture.note";
}

#[test]
fn reader_schema_admits_exactly_the_declared_classes_and_details() {
    let mut builder = AuditRegistry::builder();
    builder.declare::<Note>().unwrap();
    let registry = builder.build();
    let reader = serde_json::to_value(reader_schema(&registry).unwrap()).unwrap();
    let classes = reader.pointer("/$defs/AuditClass/enum").unwrap();
    assert_eq!(
        classes,
        &serde_json::json!([
            "audit.export",
            "audit.view",
            "fixture.note",
            "task.cancel",
            "task.update"
        ])
    );
    let fields = serde_json::json!({"$ref":"#/$defs/AuditDetailFields","$defs":reader["$defs"]});
    let validator = jsonschema::validator_for(&fields).unwrap();
    assert!(validator.is_valid(&serde_json::json!({"count": 2})));
    assert!(validator.is_valid(&serde_json::json!({})));
    assert!(!validator.is_valid(&serde_json::json!({"count": "two"})));
    assert!(!validator.is_valid(&serde_json::json!({"note": "free text"})));
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename = "installation", deny_unknown_fields)]
struct ImpersonatedCore {
    id: uuid::Uuid,
}
impl AuditTargetOwner for ImpersonatedCore {
    const KIND: &'static str = "installation";
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename = "widget", deny_unknown_fields)]
struct WrongType {
    id: uuid::Uuid,
}
impl AuditTargetOwner for WrongType {
    const KIND: &'static str = "widget";
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename = "open", deny_unknown_fields)]
struct OpenPayload {
    payload: serde_json::Value,
}
impl AuditTargetOwner for OpenPayload {
    const KIND: &'static str = "open";
}
#[test]
fn rejects_core_collision_type_substitution_and_open_payload() {
    let mut builder = AuditRegistry::builder();
    assert!(matches!(
        builder.register_target::<ImpersonatedCore>(),
        Err(AuditTargetError::Collision(_))
    ));
    assert!(matches!(
        builder.register_target::<OpenPayload>(),
        Err(AuditTargetError::Invalid)
    ));
    builder.register_target::<Widget>().unwrap_or_else(|error| {
        panic!(
            "{error}: {}",
            serde_json::to_value(schemars::schema_for!(Widget)).unwrap()
        )
    });
    let registry = builder.build();
    assert!(matches!(
        registry.target(WrongType {
            id: uuid::Uuid::new_v4()
        }),
        Err(AuditTargetError::Type)
    ));
}

fn widget_schema(schema: &mut schemars::Schema) {
    schema
        .as_object_mut()
        .unwrap()
        .get_mut("properties")
        .and_then(serde_json::Value::as_object_mut)
        .unwrap()
        .insert(
            "kind".into(),
            serde_json::json!({"type":"string","const":"widget"}),
        );
    schema
        .as_object_mut()
        .unwrap()
        .get_mut("required")
        .and_then(serde_json::Value::as_array_mut)
        .unwrap()
        .push(serde_json::json!("kind"));
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DefinitionOwner {
    id: String,
}
impl schemars::JsonSchema for DefinitionOwner {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "DefinitionOwner".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"type":"object","additionalProperties":false,"properties":{"kind":{"type":"string","const":"definition_owner"},"id":{"$ref":"#/$defs/AuditTarget"}},"required":["kind","id"],"$defs":{"AuditTarget":{"type":"string"}}})
    }
}
impl AuditTargetOwner for DefinitionOwner {
    const KIND: &'static str = "definition_owner";
}
#[test]
fn owner_definitions_cannot_replace_core_definitions() {
    assert!(
        matches!(AuditRegistry::builder().register_target::<DefinitionOwner>(),Err(AuditTargetError::SchemaCollision(name)) if name=="AuditTarget")
    );
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(transform = tuple_target_schema)]
struct TupleOwner {
    kind: TupleKind,
    values: (String,),
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum TupleKind {
    TupleOwner,
}
fn tuple_target_schema(schema: &mut schemars::Schema) {
    schema
        .as_object_mut()
        .unwrap()
        .get_mut("properties")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert(
            "kind".into(),
            serde_json::json!({"type":"string","const":"tuple_owner"}),
        );
}
impl AuditTargetOwner for TupleOwner {
    const KIND: &'static str = "tuple_owner";
}
#[test]
fn composed_tuple_schema_and_decoder_reject_unconstrained_trailing_objects() {
    let mut builder = AuditRegistry::builder();
    builder.register_target::<TupleOwner>().unwrap();
    let registry = builder.build();
    let reader = serde_json::to_value(reader_schema(&registry).unwrap()).unwrap();
    let target_schema = serde_json::json!({"$ref":"#/$defs/AuditTarget","$defs":reader["$defs"]});
    let validator = jsonschema::validator_for(&target_schema).unwrap();
    let valid = serde_json::json!({"kind":"tuple_owner","values":["first"]});
    assert!(validator.is_valid(&valid));
    registry.decoder().from_value::<AuditTarget>(valid).unwrap();
    let invalid =
        serde_json::json!({"kind":"tuple_owner","values":["first",{"arbitrary":{"nested":true}}]});
    assert!(!validator.is_valid(&invalid));
    assert!(
        registry
            .decoder()
            .from_value::<AuditTarget>(invalid)
            .is_err()
    );
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TextDefinitionOwner {
    id: String,
}
impl schemars::JsonSchema for TextDefinitionOwner {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "TextDefinitionOwner".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"type":"object","additionalProperties":false,"properties":{"kind":{"type":"string","const":"text_definition"},"id":{"$ref":"#/$defs/OwnerShared"}},"required":["kind","id"],"$defs":{"OwnerShared":{"type":"string"}}})
    }
}
impl AuditTargetOwner for TextDefinitionOwner {
    const KIND: &'static str = "text_definition";
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct IntegerDefinitionOwner {
    id: u32,
}
impl schemars::JsonSchema for IntegerDefinitionOwner {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "IntegerDefinitionOwner".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"type":"object","additionalProperties":false,"properties":{"kind":{"type":"string","const":"integer_definition"},"id":{"$ref":"#/$defs/OwnerShared"}},"required":["kind","id"],"$defs":{"OwnerShared":{"type":"integer","minimum":0,"maximum":4294967295_u64}}})
    }
}
impl AuditTargetOwner for IntegerDefinitionOwner {
    const KIND: &'static str = "integer_definition";
}
#[test]
fn owner_definitions_cannot_replace_other_owner_definitions() {
    let mut builder = AuditRegistry::builder();
    builder.register_target::<TextDefinitionOwner>().unwrap();
    assert!(
        matches!(builder.register_target::<IntegerDefinitionOwner>(), Err(AuditTargetError::SchemaCollision(name)) if name == "OwnerShared")
    );
}

#[test]
fn query_and_page_cursors_admit_current_names_and_refuse_retired_mixed_or_duplicate_keys() {
    let registry = AuditRegistry::platform();
    let decoder = registry.decoder();
    let cursor = AuditCursor {
        order: AuditOrder::OldestFirst,
        partition: AuditPartition::Installation,
        last_id: AuditRecordId::new(),
    };
    let mut query = AuditQuery::new(AuditPartition::Installation);
    query.cursor = Some(cursor.clone());
    let current = serde_json::to_value(&query).unwrap();
    assert_eq!(current["cursor"]["lastId"], cursor.last_id.to_string());
    assert!(current["cursor"].get("last_id").is_none());
    let admitted: AuditQuery = decoder.from_value(current.clone()).unwrap();
    assert_eq!(admitted.cursor, Some(cursor.clone()));
    let page = serde_json::json!({"records": [], "next": current["cursor"]});
    let admitted: AuditPage = decoder.from_value(page.clone()).unwrap();
    assert_eq!(admitted.next, Some(cursor.clone()));
    for (root, field) in [(current.clone(), "cursor"), (page, "next")] {
        let mut retired = root.clone();
        let object = retired[field].as_object_mut().unwrap();
        let id = object.remove("lastId").unwrap();
        object.insert("last_id".into(), id.clone());
        let mut mixed = root.clone();
        mixed[field]["last_id"] = id;
        let text = serde_json::to_string(&root).unwrap();
        let duplicate = text.replace(
            "\"lastId\":",
            &format!("\"lastId\":\"{}\",\"lastId\":", cursor.last_id),
        );
        if field == "cursor" {
            assert!(decoder.from_value::<AuditQuery>(retired).is_err());
            assert!(decoder.from_value::<AuditQuery>(mixed).is_err());
            assert!(decoder.from_str::<AuditQuery>(&duplicate).is_err());
        } else {
            assert!(decoder.from_value::<AuditPage>(retired).is_err());
            assert!(decoder.from_value::<AuditPage>(mixed).is_err());
            assert!(decoder.from_str::<AuditPage>(&duplicate).is_err());
        }
    }
    let mut wrong_order = current.clone();
    wrong_order["cursor"]["order"] = serde_json::json!("newest_first");
    assert!(decoder.from_value::<AuditQuery>(wrong_order).is_err());
    let mut wrong_partition = current;
    wrong_partition["cursor"]["partition"] = serde_json::json!({"kind":"tenant", "tenant":"other"});
    assert!(decoder.from_value::<AuditQuery>(wrong_partition).is_err());
    let schema = serde_json::to_value(reader_schema(&registry).unwrap()).unwrap();
    let properties = &schema["$defs"]["AuditCursor"]["properties"];
    assert!(properties.get("lastId").is_some());
    assert!(properties.get("last_id").is_none());
}
