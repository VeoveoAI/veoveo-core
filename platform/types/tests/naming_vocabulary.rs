//! The naming vocabulary's meta-schema is generated from the `NamingProfile` type.
use serde_json::{Value, json};
use std::path::Path;
use veoveo_types::{NAMING_META_SCHEMA, NAMING_PROFILE_KEY, NAMING_VOCABULARY, NamingProfile};

fn meta_schema() -> Value {
    let mut profile = serde_json::to_value(schemars::schema_for!(NamingProfile)).unwrap();
    let definitions = profile
        .as_object_mut()
        .unwrap()
        .remove("$defs")
        .unwrap_or_else(|| json!({}));
    profile.as_object_mut().unwrap().remove("$schema");
    let mut defs = definitions.as_object().unwrap().clone();
    defs.insert("NamingProfile".into(), profile);
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": NAMING_META_SCHEMA,
        "$vocabulary": {
            "https://json-schema.org/draft/2020-12/vocab/core": true,
            "https://json-schema.org/draft/2020-12/vocab/applicator": true,
            "https://json-schema.org/draft/2020-12/vocab/unevaluated": true,
            "https://json-schema.org/draft/2020-12/vocab/validation": true,
            "https://json-schema.org/draft/2020-12/vocab/meta-data": true,
            "https://json-schema.org/draft/2020-12/vocab/format-annotation": true,
            "https://json-schema.org/draft/2020-12/vocab/content": true,
            NAMING_VOCABULARY: false
        },
        "$dynamicAnchor": "meta",
        "title": "Veoveo naming vocabulary",
        "allOf": [{"$ref": "https://json-schema.org/draft/2020-12/schema"}],
        "properties": {NAMING_PROFILE_KEY: {"$ref": "#/$defs/NamingProfile"}},
        "$defs": defs
    })
}

#[test]
fn published_meta_schema_matches_the_naming_profile() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas/naming.meta.json");
    let actual = meta_schema();
    if std::env::var_os("VEOVEO_CAPTURE_CATALOG").is_some() {
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&actual).unwrap()),
        )
        .unwrap();
        return;
    }
    let expected: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(actual, expected, "schemas/naming.meta.json is stale");
}
