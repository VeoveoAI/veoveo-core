//! Audit classes. Each owner declares the classes it records as `<owner>.<name>`, such
//! as `task.update`: the owner is the declaring component's name and the name a
//! snake_case action.
use veoveo_types::{IdMetadata, IdProfile, IdProfileSpec, IdentifierError};

/// The longest owner, the server slug limit.
const MAX_OWNER: usize = 48;
/// The longest name after the owner.
const MAX_NAME: usize = 63;

/// The owner-declared class of an audit record, `<owner>.<name>`. The owner follows
/// server slug syntax, a lowercase letter followed by lowercase letters, digits and
/// hyphens, and the name is a lowercase letter followed by lowercase letters, digits and
/// underscores.
#[cfg_attr(
    feature = "postgres",
    veoveo_types::id(text(AuditClassProfile), postgres)
)]
#[cfg_attr(not(feature = "postgres"), veoveo_types::id(text(AuditClassProfile)))]
pub struct AuditClass(String);

impl AuditClass {
    /// The component that declared the class.
    pub fn owner(&self) -> &str {
        self.parts().0
    }

    /// The action the class names within its owner.
    pub fn name(&self) -> &str {
        self.parts().1
    }

    fn parts(&self) -> (&str, &str) {
        self.as_str()
            .split_once('.')
            .expect("an admitted audit class has an owner and a name")
    }
}

fn validate_class(value: &str, _: IdMetadata) -> Result<(), IdentifierError> {
    let Some((owner, name)) = value.split_once('.') else {
        return Err(IdentifierError::new(value, "must be `<owner>.<name>`"));
    };
    let first = |part: &str| part.bytes().next().is_some_and(|b| b.is_ascii_lowercase());
    if owner.len() > MAX_OWNER
        || !first(owner)
        || !owner
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(IdentifierError::new(
            value,
            "owner must be a lowercase letter followed by at most 47 lowercase letters, digits and hyphens",
        ));
    }
    if name.len() > MAX_NAME
        || !first(name)
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(IdentifierError::new(
            value,
            "name must be a lowercase letter followed by at most 62 lowercase letters, digits and underscores",
        ));
    }
    Ok(())
}

fn class_schema(_: &mut schemars::SchemaGenerator, _: IdMetadata) -> schemars::Schema {
    schemars::json_schema!({
        "type": "string",
        "pattern": "^[a-z][a-z0-9-]{0,47}\\.[a-z][a-z0-9_]{0,62}$"
    })
}

#[doc(hidden)]
pub struct AuditClassProfile;
impl IdProfile for AuditClassProfile {
    type Error = IdentifierError;
    const PROFILE: IdProfileSpec<Self::Error> =
        IdProfileSpec::text(validate_class).owner_schema(class_schema, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_name_an_owner_and_a_snake_case_action() {
        let class = AuditClass::parse("time-zones.event_create").unwrap();
        assert_eq!(class.owner(), "time-zones");
        assert_eq!(class.name(), "event_create");
        let longest = format!("{}.{}", "a".repeat(MAX_OWNER), "b".repeat(MAX_NAME));
        assert!(AuditClass::parse(&longest).is_ok());
        for invalid in [
            "task",
            ".update",
            "task.",
            "Task.update",
            "task.Update",
            "task.up-date",
            "task_list.update",
            "1task.update",
            "task.1update",
            "task.update.more",
            &format!("{}.b", "a".repeat(MAX_OWNER + 1)),
            &format!("a.{}", "b".repeat(MAX_NAME + 1)),
        ] {
            assert!(AuditClass::parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn the_schema_matches_the_admission_grammar() {
        let schema = serde_json::to_value(schemars::schema_for!(AuditClass)).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        for value in ["task.update", "time-zones.event_create"] {
            assert!(validator.is_valid(&serde_json::json!(value)));
        }
        for value in ["task", "Task.update", "task.up-date"] {
            assert!(!validator.is_valid(&serde_json::json!(value)));
        }
    }
}
