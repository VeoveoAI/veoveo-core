//! The installation's tenants and principals, keyed by deterministic UUIDv5 identifiers
//! derived from their keys.
use uuid::Uuid;

use crate::{PlatformStore, StoreError};

/// The namespace of every deterministic platform identifier.
const PLATFORM_ID_NAMESPACE: Uuid = Uuid::from_u128(0x7f7b11e2_3b9a_5c7a_9d51_2cf8e1bdfab4);

/// SQLSTATE that `identity.ensure_identity` raises when a stored tenant disagrees with
/// its key.
const TENANT_CONFLICT: &str = "VV001";
/// SQLSTATE that `identity.ensure_identity` raises when a stored principal disagrees
/// with its key.
const PRINCIPAL_CONFLICT: &str = "VV002";

#[doc(hidden)]
pub struct PersistenceIds;
impl veoveo_types::IdProfile for PersistenceIds {
    type Error = uuid::Error;
    const PROFILE: veoveo_types::IdProfileSpec<Self::Error> = veoveo_types::IdProfileSpec {
        wire: veoveo_types::IdWire::InnerUuid,
        schema: veoveo_types::IdSchema::DerivedUuid,
        ..veoveo_types::IdProfileSpec::generated_uuid(
            veoveo_types::UuidGrammar {
                versions: &[],
                variant: veoveo_types::UuidVariant::Any,
                spelling: veoveo_types::UuidSpelling::ParserAliases,
            },
            persistence_id_error,
        )
    };
}

fn persistence_id_error(
    value: &str,
    _: veoveo_types::IdMetadata,
    _: veoveo_types::IdFailure,
) -> uuid::Error {
    Uuid::parse_str(value)
        .expect_err("only malformed UUIDs reach the unrestricted persistence error mapping")
}

/// `identity.enterprise.id`
#[veoveo_types::id(uuid(PersistenceIds), const_uuid, postgres)]
pub struct EnterpriseId(Uuid);

/// `identity.tenant.id`
#[veoveo_types::id(uuid(PersistenceIds), const_uuid, postgres)]
pub struct TenantId(Uuid);

/// `identity.principal.id`
#[veoveo_types::id(uuid(PersistenceIds), const_uuid, postgres)]
pub struct PrincipalId(Uuid);

/// `identity.work_context.id`
#[veoveo_types::id(uuid(PersistenceIds), const_uuid, postgres)]
pub struct WorkContextId(Uuid);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, veoveo_types::Vocabulary)]
#[vocabulary(postgres)]
pub enum PrincipalKind {
    #[vocabulary(rename = "user")]
    User,
    #[vocabulary(rename = "service")]
    Service,
}

/// A principal and its tenant, by identifier and by key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformIdentity {
    pub tenant_id: TenantId,
    pub principal_id: PrincipalId,
    pub tenant_key: String,
    pub principal_key: String,
}

pub fn deterministic_enterprise_id() -> EnterpriseId {
    EnterpriseId::from_uuid(Uuid::new_v5(&PLATFORM_ID_NAMESPACE, b"veoveo-installation"))
}

pub fn deterministic_tenant_id(tenant_key: &str) -> Result<TenantId, StoreError> {
    validate_identity_field("tenant_key", tenant_key, 256)?;
    Ok(TenantId::from_uuid(Uuid::new_v5(
        &PLATFORM_ID_NAMESPACE,
        format!("tenant:{tenant_key}").as_bytes(),
    )))
}

pub fn deterministic_principal_id(
    tenant_key: &str,
    principal_key: &str,
) -> Result<PrincipalId, StoreError> {
    validate_identity_field("tenant_key", tenant_key, 256)?;
    validate_identity_field("principal_key", principal_key, 512)?;
    Ok(PrincipalId::from_uuid(Uuid::new_v5(
        &PLATFORM_ID_NAMESPACE,
        format!("principal:{tenant_key}:{principal_key}").as_bytes(),
    )))
}

pub fn deterministic_work_context_id(
    tenant_key: &str,
    context_key: &str,
) -> Result<WorkContextId, StoreError> {
    validate_identity_field("tenant_key", tenant_key, 256)?;
    validate_identity_field("context_key", context_key, 256)?;
    Ok(WorkContextId::from_uuid(Uuid::new_v5(
        &PLATFORM_ID_NAMESPACE,
        format!("work-context:{tenant_key}:{context_key}").as_bytes(),
    )))
}

impl PlatformStore {
    /// Record the principal and its tenant. A new principal's display name is the last
    /// segment of its subject; an existing principal keeps its display name.
    pub async fn ensure_identity(
        &self,
        tenant_key: &str,
        principal_key: &str,
        issuer: &str,
        subject: &str,
        kind: PrincipalKind,
    ) -> Result<PlatformIdentity, StoreError> {
        self.record_identity(tenant_key, principal_key, issuer, subject, kind, None)
            .await
    }

    /// Record the principal and its tenant with the display name the identity provider
    /// asserted. The display name is presentation only and never takes part in matching.
    pub async fn ensure_named_identity(
        &self,
        tenant_key: &str,
        principal_key: &str,
        issuer: &str,
        subject: &str,
        kind: PrincipalKind,
        display_name: &str,
    ) -> Result<PlatformIdentity, StoreError> {
        validate_identity_field("display_name", display_name, 512)?;
        self.record_identity(
            tenant_key,
            principal_key,
            issuer,
            subject,
            kind,
            Some(display_name),
        )
        .await
    }

    async fn record_identity(
        &self,
        tenant_key: &str,
        principal_key: &str,
        issuer: &str,
        subject: &str,
        kind: PrincipalKind,
        display_name: Option<&str>,
    ) -> Result<PlatformIdentity, StoreError> {
        validate_identity_field("issuer", issuer, 2_048)?;
        validate_identity_field("subject", subject, 2_048)?;
        let enterprise_id = deterministic_enterprise_id();
        let tenant_id = deterministic_tenant_id(tenant_key)?;
        let principal_id = deterministic_principal_id(tenant_key, principal_key)?;
        let fallback = compact_identity_label(subject);
        sqlx::query_file!(
            "src/identity/ensure_identity.sql",
            enterprise_id as EnterpriseId,
            tenant_id as TenantId,
            tenant_key,
            principal_id as PrincipalId,
            principal_key,
            kind as PrincipalKind,
            issuer,
            subject,
            display_name,
            fallback,
        )
        .execute(self.pool())
        .await
        .map_err(|error| identity_conflict(error, tenant_key, principal_key))?;
        Ok(PlatformIdentity {
            tenant_id,
            principal_id,
            tenant_key: tenant_key.to_owned(),
            principal_key: principal_key.to_owned(),
        })
    }
}

fn identity_conflict(error: sqlx::Error, tenant_key: &str, principal_key: &str) -> StoreError {
    let Some(database) = error.as_database_error() else {
        return error.into();
    };
    let principal_unique = database.is_unique_violation()
        && database
            .constraint()
            .is_some_and(|constraint| constraint.starts_with("principal_tenant_"));
    match database.code().as_deref() {
        Some(TENANT_CONFLICT) => StoreError::IdentityConflict {
            entity: "tenant",
            key: tenant_key.to_owned(),
        },
        Some(PRINCIPAL_CONFLICT) => StoreError::IdentityConflict {
            entity: "principal",
            key: principal_key.to_owned(),
        },
        _ if principal_unique => StoreError::IdentityConflict {
            entity: "principal",
            key: principal_key.to_owned(),
        },
        _ => error.into(),
    }
}

fn validate_identity_field(
    field: &'static str,
    value: &str,
    max_bytes: usize,
) -> Result<(), StoreError> {
    if value.trim().is_empty() {
        return Err(StoreError::InvalidIdentityField {
            field,
            reason: "must not be empty",
        });
    }
    if value.len() > max_bytes {
        return Err(StoreError::InvalidIdentityField {
            field,
            reason: "exceeds maximum encoded length",
        });
    }
    if value.chars().any(char::is_control) {
        return Err(StoreError::InvalidIdentityField {
            field,
            reason: "must not contain control characters",
        });
    }
    Ok(())
}

/// The last nonempty `/` or `#` segment of a subject, at most 128 characters.
fn compact_identity_label(subject: &str) -> String {
    let candidate = subject
        .split(['#', '/'])
        .rfind(|segment| !segment.trim().is_empty())
        .unwrap_or(subject)
        .trim();
    if candidate.is_empty() {
        return "Principal".to_owned();
    }
    candidate.chars().take(128).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principal_identity_is_tenant_scoped_and_stable() {
        let acme_alice = deterministic_principal_id("acme", "alice").unwrap();
        let beta_alice = deterministic_principal_id("beta", "alice").unwrap();
        assert_ne!(
            deterministic_tenant_id("acme").unwrap(),
            deterministic_tenant_id("beta").unwrap()
        );
        assert_ne!(acme_alice, beta_alice);
        assert_eq!(
            acme_alice,
            deterministic_principal_id("acme", "alice").unwrap()
        );
        assert_eq!(acme_alice.as_uuid().get_version_num(), 5);
    }

    #[test]
    fn rejects_blank_long_and_control_identity_material() {
        for (value, reason) in [
            ("  ", "must not be empty"),
            ("a\u{7}b", "must not contain control characters"),
        ] {
            assert!(matches!(
                deterministic_tenant_id(value),
                Err(StoreError::InvalidIdentityField { field: "tenant_key", reason: r }) if r == reason
            ));
        }
        assert!(deterministic_tenant_id(&"t".repeat(257)).is_err());
    }

    #[test]
    fn compact_labels_use_the_last_subject_segment() {
        assert_eq!(
            compact_identity_label("https://issuer.example/users/alice"),
            "alice"
        );
        assert_eq!(compact_identity_label("https://issuer.example#bob"), "bob");
        assert_eq!(compact_identity_label("   "), "Principal");
    }
}
