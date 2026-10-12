//! Server-declared permission and resource-type names. A registered server owns every
//! name whose first segment is its slug; the Gateway checks that ownership at registration.
use crate::{IdentifierError, ServerSlug};

#[doc = "Permission a registered server declares, `{slug}.{segment}`, for example `time.events.write`."]
#[veoveo_types::id(text(PermissionNames))]
pub struct PermissionName(String);
#[doc = "Resource type a permission acts on, `{slug}.{segment}`, for example `time.event`."]
#[veoveo_types::id(text(PermissionNames))]
pub struct ResourceTypeName(String);

const MAX_LENGTH: usize = 128;

/// Lowercase dotted segments, at least two, each starting with a letter.
fn validate_dotted(value: &str) -> Result<(), IdentifierError> {
    if value.len() > MAX_LENGTH {
        return Err(IdentifierError::new(value, "must be at most 128 bytes"));
    }
    let mut segments = 0;
    for segment in value.split('.') {
        segments += 1;
        let mut bytes = segment.bytes();
        let Some(first) = bytes.next() else {
            return Err(IdentifierError::new(
                value,
                "must not contain empty segments",
            ));
        };
        if !first.is_ascii_lowercase() {
            return Err(IdentifierError::new(
                value,
                "each segment must start with a lowercase ASCII letter",
            ));
        }
        if !bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_') {
            return Err(IdentifierError::new(
                value,
                "segments contain only lowercase ASCII letters, digits, hyphen or underscore",
            ));
        }
    }
    if segments < 2 {
        return Err(IdentifierError::new(
            value,
            "must name its server namespace and at least one more segment",
        ));
    }
    Ok(())
}

fn namespace(value: &str) -> &str {
    value
        .split_once('.')
        .map_or(value, |(namespace, _)| namespace)
}

impl PermissionName {
    /// The first segment, which must equal the declaring server's slug.
    pub fn namespace(&self) -> &str {
        namespace(self.as_str())
    }
    pub fn is_owned_by(&self, server: &ServerSlug) -> bool {
        self.namespace() == server.as_str()
    }
}

impl ResourceTypeName {
    /// The first segment, which must equal the declaring server's slug.
    pub fn namespace(&self) -> &str {
        namespace(self.as_str())
    }
    pub fn is_owned_by(&self, server: &ServerSlug) -> bool {
        self.namespace() == server.as_str()
    }
}

#[doc(hidden)]
pub struct PermissionNames;
impl crate::IdProfile for PermissionNames {
    type Error = IdentifierError;
    const PROFILE: crate::IdProfileSpec<Self::Error> =
        crate::IdProfileSpec::text(|value, _| validate_dotted(value));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_names_admit_owner_segments() {
        let permission = PermissionName::parse("time.events.write").unwrap();
        assert_eq!(permission.namespace(), "time");
        assert!(permission.is_owned_by(&ServerSlug::parse("time").unwrap()));
        assert!(!permission.is_owned_by(&ServerSlug::parse("timeseries").unwrap()));
        let resource = ResourceTypeName::parse("time.event").unwrap();
        assert!(resource.is_owned_by(&ServerSlug::parse("time").unwrap()));
    }

    #[test]
    fn malformed_names_are_rejected() {
        for value in [
            "",
            "time",
            "time.",
            ".read",
            "time..read",
            "Time.read",
            "time.Read",
            "time.1read",
            "time.re ad",
            "time:read",
        ] {
            assert!(PermissionName::parse(value).is_err(), "{value}");
            assert!(ResourceTypeName::parse(value).is_err(), "{value}");
        }
        assert!(PermissionName::parse(format!("time.{}", "a".repeat(124))).is_err());
        assert!(PermissionName::parse(format!("time.{}", "a".repeat(123))).is_ok());
    }

    #[test]
    fn names_round_trip_through_serde() {
        let permission = PermissionName::parse("time.read").unwrap();
        let wire = serde_json::to_value(&permission).unwrap();
        assert_eq!(wire, "time.read");
        assert_eq!(
            serde_json::from_value::<PermissionName>(wire).unwrap(),
            permission
        );
        assert!(serde_json::from_value::<PermissionName>(serde_json::json!("read")).is_err());
    }
}
