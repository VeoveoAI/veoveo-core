//! Appends audit records inside a caller's PostgreSQL transaction.
//!
//! Domain code that changes state and records it does both in one transaction: it
//! admits its drafts against the installation's target registry, runs its own writes,
//! calls [`AuditTransactionWrite::append`] on the same connection and commits.
use crate::{AuditClass, AuditDraft, AuditTargetError, AuditTargetRegistry};
use chrono::{DateTime, SubsecRound, Utc};
use sqlx::PgConnection;

/// The most drafts one append admits.
pub const AUDIT_APPEND_LIMIT: usize = 4096;

/// SQLSTATE that `audit.append` raises when a stored record and a draft share an
/// identity but differ in content.
pub(crate) const IDENTITY_CONFLICT: &str = "VV501";

#[derive(Debug, thiserror::Error)]
pub enum AuditAppendError {
    #[error(transparent)]
    Target(#[from] AuditTargetError),
    #[error("an audit append admits at most {AUDIT_APPEND_LIMIT} records")]
    BatchLimit,
    #[error("an audit record identity already holds different content")]
    IdentityConflict,
    #[error("audit append failed: {0}")]
    Database(sqlx::Error),
}

impl From<sqlx::Error> for AuditAppendError {
    fn from(error: sqlx::Error) -> Self {
        let conflict = error
            .as_database_error()
            .and_then(|database| database.code())
            .is_some_and(|code| code == IDENTITY_CONFLICT);
        if conflict {
            Self::IdentityConflict
        } else {
            Self::Database(error)
        }
    }
}

/// Drafts admitted against one target registry, ready to append inside an existing
/// transaction. Appending starts no transaction: the records commit or roll back with
/// the caller's own writes. Repeating an append of the same drafts changes nothing; a
/// draft that reuses a stored identity with other content fails the append, and the
/// caller's transaction must then roll back.
#[derive(Debug, Clone, Default)]
pub struct AuditTransactionWrite {
    drafts: Vec<serde_json::Value>,
    classes: Vec<AuditClass>,
    occurred: Vec<DateTime<Utc>>,
}

impl AuditTransactionWrite {
    pub fn new(
        registry: &AuditTargetRegistry,
        draft: AuditDraft,
    ) -> Result<Self, AuditAppendError> {
        Self::batch(registry, [draft])
    }

    pub fn batch(
        registry: &AuditTargetRegistry,
        drafts: impl IntoIterator<Item = AuditDraft>,
    ) -> Result<Self, AuditAppendError> {
        let mut write = Self::default();
        for draft in drafts {
            write.push(registry, &draft)?;
        }
        Ok(write)
    }

    /// Admit one more draft.
    pub fn push(
        &mut self,
        registry: &AuditTargetRegistry,
        draft: &AuditDraft,
    ) -> Result<(), AuditAppendError> {
        if self.drafts.len() == AUDIT_APPEND_LIMIT {
            return Err(AuditAppendError::BatchLimit);
        }
        registry.validate(draft.target())?;
        self.drafts
            .push(serde_json::to_value(draft).expect("an admitted draft serializes"));
        self.classes.push(draft.detail().class());
        self.occurred.push(stored_time(draft.occurred_at()));
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.drafts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.drafts.is_empty()
    }

    /// Append on the caller's connection, normally inside its open transaction.
    pub async fn append(&self, connection: &mut PgConnection) -> Result<(), AuditAppendError> {
        if self.drafts.is_empty() {
            return Ok(());
        }
        sqlx::query_file!(
            "src/postgres/append.sql",
            &self.drafts,
            &self.classes as &[AuditClass],
            &self.occurred,
        )
        .execute(connection)
        .await?;
        Ok(())
    }
}

/// PostgreSQL stores microseconds. A draft keeps its own time; the stored occurrence
/// column holds that time truncated to microseconds.
pub fn stored_time(time: DateTime<Utc>) -> DateTime<Utc> {
    time.trunc_subsecs(6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AuditDetail, AuditOutcome, AuditReadMethod, AuditReason, AuditRecordId, AuditRequest,
        AuditTarget, AuditTraceId,
    };
    use sqlx::{Postgres, Type, postgres::PgHasArrayType};

    #[test]
    fn vocabularies_bind_as_text_and_identities_by_their_storage() {
        let text = <String as Type<Postgres>>::type_info();
        assert_eq!(<AuditClass as Type<Postgres>>::type_info(), text);
        assert_eq!(<AuditOutcome as Type<Postgres>>::type_info(), text);
        assert_eq!(<AuditTraceId as Type<Postgres>>::type_info(), text);
        assert_eq!(
            <AuditClass as PgHasArrayType>::array_type_info(),
            <String as PgHasArrayType>::array_type_info()
        );
        assert_eq!(
            <AuditRecordId as Type<Postgres>>::type_info(),
            <uuid::Uuid as Type<Postgres>>::type_info()
        );
    }

    #[test]
    fn admission_checks_the_registry_and_keeps_microseconds() {
        let draft = AuditDraft::builder(
            AuditRequest::background(),
            AuditTarget::Installation,
            AuditDetail::Read {
                method: AuditReadMethod::Status,
            },
            AuditOutcome::Allowed,
            AuditReason::Accepted,
        )
        .occurred_at("2026-10-12T01:02:03.123456789Z".parse().unwrap())
        .build()
        .unwrap();
        let write = AuditTransactionWrite::new(&AuditTargetRegistry::empty(), draft).unwrap();
        assert_eq!(write.len(), 1);
        assert_eq!(write.classes, [AuditClass::ApiActivity]);
        assert_eq!(
            write.occurred,
            ["2026-10-12T01:02:03.123456Z"
                .parse::<DateTime<Utc>>()
                .unwrap()]
        );
        assert!(AuditTransactionWrite::default().is_empty());
    }
}
