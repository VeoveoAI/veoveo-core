//! JSON Schema for manifest names. Each pattern states the same rule as the name's
//! checked constructor, which stays the admission authority.
use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};

use crate::*;

const IDENTIFIER: &str = "^[a-z][a-z0-9_]*$";
const FUNCTION: &str = "^fn::[a-z][a-z0-9_]*(::[a-z][a-z0-9_]*)*$";
const FUNCTION_PREFIX: &str = "^fn::[a-z][a-z0-9_]*(::[a-z][a-z0-9_]*)*(::)?$";
const EXTENSION: &str = "^[A-Za-z0-9_.:/-]+$";

fn text(pattern: &str, max_length: u32) -> Schema {
    json_schema!({"type": "string", "pattern": pattern, "minLength": 1, "maxLength": max_length})
}

impl JsonSchema for ModuleName {
    fn schema_name() -> Cow<'static, str> {
        "ModuleName".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        text("^[a-z][a-z0-9_-]*$", 64)
    }
}
impl JsonSchema for TableName {
    fn schema_name() -> Cow<'static, str> {
        "TableName".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        text(IDENTIFIER, 128)
    }
}
impl JsonSchema for TablePrefix {
    fn schema_name() -> Cow<'static, str> {
        "TablePrefix".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        text(IDENTIFIER, 128)
    }
}
impl JsonSchema for AnalyzerName {
    fn schema_name() -> Cow<'static, str> {
        "AnalyzerName".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        text(IDENTIFIER, 128)
    }
}
impl JsonSchema for MigrationName {
    fn schema_name() -> Cow<'static, str> {
        "MigrationName".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        text(IDENTIFIER, 128)
    }
}
impl JsonSchema for FunctionName {
    fn schema_name() -> Cow<'static, str> {
        "FunctionName".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        text(FUNCTION, 512)
    }
}
impl JsonSchema for FunctionPrefix {
    fn schema_name() -> Cow<'static, str> {
        "FunctionPrefix".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        text(FUNCTION_PREFIX, 514)
    }
}
impl JsonSchema for ExtensionPointName {
    fn schema_name() -> Cow<'static, str> {
        "ExtensionPointName".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        text(EXTENSION, 256)
    }
}
impl JsonSchema for ExtensionImplementationName {
    fn schema_name() -> Cow<'static, str> {
        "ExtensionImplementationName".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        text(EXTENSION, 256)
    }
}
impl JsonSchema for MigrationVersion {
    fn schema_name() -> Cow<'static, str> {
        "MigrationVersion".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type": "integer", "minimum": 0, "maximum": u32::MAX})
    }
}
