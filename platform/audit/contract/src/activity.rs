//! How a recorded action ended and why. What happened is the record's owner-declared
//! class and detail; the reason is a code its owner chooses.
use veoveo_types::{IdMetadata, IdProfile, IdProfileSpec, IdentifierError};

/// The outcome of a recorded action. OCSF maps it to `status_id`, Success, Failure or
/// Unknown, and the Security Control profile's `action_id`, Allowed or Denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, veoveo_types::Vocabulary)]
#[cfg_attr(feature = "postgres", vocabulary(postgres))]
pub enum AuditOutcome {
    Success,
    Denied,
    Failure,
    Unknown,
}

/// The longest reason code.
pub const MAX_REASON_BYTES: usize = 64;

/// Why an action ended as it did, a code the record's owner defines, such as
/// `token_audience_mismatch` or `quota_exceeded`: a lowercase letter followed by at most
/// 63 lowercase letters, digits and underscores. A reason reads within its record's
/// source, so two owners may use the same code.
#[cfg_attr(
    feature = "postgres",
    veoveo_types::id(text(AuditReasonProfile), postgres)
)]
#[cfg_attr(not(feature = "postgres"), veoveo_types::id(text(AuditReasonProfile)))]
pub struct AuditReason(String);

fn validate_reason(value: &str, _: IdMetadata) -> Result<(), IdentifierError> {
    let mut bytes = value.bytes();
    let valid = value.len() <= MAX_REASON_BYTES
        && bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if valid {
        Ok(())
    } else {
        Err(IdentifierError::new(
            value,
            "must be a lowercase letter followed by at most 63 lowercase letters, digits and underscores",
        ))
    }
}

fn reason_schema(_: &mut schemars::SchemaGenerator, _: IdMetadata) -> schemars::Schema {
    schemars::json_schema!({"type": "string", "pattern": "^[a-z][a-z0-9_]{0,63}$"})
}

#[doc(hidden)]
pub struct AuditReasonProfile;
impl IdProfile for AuditReasonProfile {
    type Error = IdentifierError;
    const PROFILE: IdProfileSpec<Self::Error> =
        IdProfileSpec::text(validate_reason).owner_schema(reason_schema, false);
}

#[cfg(test)]
mod vocabulary_baseline;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons_are_short_snake_case_codes() {
        assert!(AuditReason::parse("token_audience_mismatch").is_ok());
        assert!(AuditReason::parse("a".repeat(MAX_REASON_BYTES)).is_ok());
        for invalid in [
            "",
            "Quota",
            "quota-exceeded",
            "1quota",
            "quota exceeded",
            &"a".repeat(MAX_REASON_BYTES + 1),
        ] {
            assert!(AuditReason::parse(invalid).is_err(), "{invalid}");
        }
    }
}
