//! The installation's usage ledger in PostgreSQL: each server declares its kinds of
//! usage and appends estimates and actual amounts inside its own transactions, and
//! readers total a Task's cost or a tenant's days. Records follow the columns of the
//! FinOps Open Cost and Usage Specification (FOCUS) 1.4.
use chrono::NaiveDate;
use serde_json::Value;
use sqlx::PgConnection;
use veoveo_store::PlatformStore;
use veoveo_types::{PrincipalId, TaskId, TenantId};
pub use veoveo_usage_contract::*;

/// Why a usage write or read failed.
#[derive(Debug, thiserror::Error)]
pub enum UsageStoreError {
    /// A kind lies outside the writer's prefix or belongs to another owner.
    #[error("usage kind belongs to another owner: {0}")]
    ForeignKind(String),
    /// A repeated source identifier carried a different record.
    #[error("usage source identifier already holds a different record: {0}")]
    Conflict(String),
    /// The database refused a value: an undeclared kind, money that does not match the
    /// kind, or too many records at once.
    #[error("usage refused: {0}")]
    Refused(String),
    #[error("stored usage is invalid: {0}")]
    Invalid(String),
    #[error(transparent)]
    Database(sqlx::Error),
}

impl From<sqlx::Error> for UsageStoreError {
    fn from(error: sqlx::Error) -> Self {
        let Some(database) = error.as_database_error() else {
            return Self::Database(error);
        };
        let message = database.message().to_owned();
        match database.code().as_deref() {
            Some("42501") => Self::ForeignKind(message),
            Some("VV601") => Self::Conflict(message),
            Some("22023" | "23503" | "23514") => Self::Refused(message),
            _ => Self::Database(error),
        }
    }
}

fn stored<T, E: std::fmt::Display>(
    field: &str,
    result: Result<T, E>,
) -> Result<T, UsageStoreError> {
    result.map_err(|error| UsageStoreError::Invalid(format!("{field}: {error}")))
}

/// The usage ledger as the installation owner or a server reads and writes it.
#[derive(Clone)]
pub struct UsageStore {
    store: PlatformStore,
}

impl UsageStore {
    pub fn new(store: PlatformStore) -> Self {
        Self { store }
    }

    /// Declare usage kinds as the installation owner. Repeating a declaration changes
    /// nothing, and a changed declaration replaces the owner's own one.
    pub async fn declare(&self, kinds: &[UsageKindDeclaration]) -> Result<(), UsageStoreError> {
        let (names, categories, units, monetary) = columns(kinds);
        sqlx::query_file!(
            "queries/declare.sql",
            &names,
            &categories,
            &units,
            &monetary
        )
        .execute(self.store.pool())
        .await?;
        Ok(())
    }

    /// Declare the calling server's usage kinds through its own login. Every kind must
    /// lie under the prefix of one of the login's servers.
    pub async fn declare_as_server(
        &self,
        kinds: &[UsageKindDeclaration],
    ) -> Result<(), UsageStoreError> {
        let (names, categories, units, monetary) = columns(kinds);
        sqlx::query_file!(
            "queries/declare_server.sql",
            &names,
            &categories,
            &units,
            &monetary
        )
        .fetch_one(self.store.pool())
        .await?;
        Ok(())
    }

    /// Append records as the installation owner in a transaction of their own.
    pub async fn append(&self, drafts: &[UsageDraft]) -> Result<u64, UsageStoreError> {
        let mut connection = self.store.pool().acquire().await?;
        UsageWrite::append(&mut connection, drafts).await
    }

    /// Append the calling server's records through its own login in a transaction of
    /// their own.
    pub async fn append_as_server(&self, drafts: &[UsageDraft]) -> Result<u64, UsageStoreError> {
        let mut connection = self.store.pool().acquire().await?;
        UsageWrite::append_as_server(&mut connection, drafts).await
    }

    /// A Task's cost in `tenant`, per kind and currency: its actual records of a kind
    /// when it has any, else its estimates of that kind. A server reads only its own
    /// records.
    pub async fn task_usage(
        &self,
        tenant: &TenantId,
        task: TaskId,
    ) -> Result<Vec<TaskUsage>, UsageStoreError> {
        sqlx::query_file!("queries/task_usage.sql", tenant.as_str(), task.as_uuid())
            .fetch_all(self.store.pool())
            .await?
            .into_iter()
            .map(|row| {
                Ok(TaskUsage {
                    kind: stored("kind", UsageKind::parse(row.kind))?,
                    unit: stored("unit", UsageUnit::parse(row.unit))?,
                    phase: stored("phase", row.phase.parse())?,
                    quantity: stored("quantity", Quantity::parse(row.quantity))?,
                    money: money(&row.currency, row.amount)?,
                })
            })
            .collect()
    }

    /// A tenant's daily totals for the days from `from` up to `to`, at most `limit`
    /// rows, in day, principal, kind, phase and currency order.
    pub async fn daily(
        &self,
        tenant: &TenantId,
        from: NaiveDate,
        to: NaiveDate,
        limit: u16,
    ) -> Result<Vec<DailyUsage>, UsageStoreError> {
        sqlx::query_file!(
            "queries/daily.sql",
            tenant.as_str(),
            from,
            to,
            i64::from(limit.clamp(1, 10_000))
        )
        .fetch_all(self.store.pool())
        .await?
        .into_iter()
        .map(|row| {
            Ok(DailyUsage {
                day: row.day,
                principal: stored("principal", PrincipalId::parse(row.principal))?,
                kind: stored("kind", UsageKind::parse(row.kind))?,
                phase: stored("phase", row.phase.parse())?,
                quantity: stored("quantity", Quantity::parse(row.quantity))?,
                money: money(&row.currency, Some(row.amount))?,
                records: stored("record count", u64::try_from(row.records))?,
            })
        })
        .collect()
    }

    /// Create the monthly record partitions from the current month through the month of
    /// `through`, as the installation owner. Returns how many it created.
    pub async fn prepare_partitions(&self, through: NaiveDate) -> Result<u32, UsageStoreError> {
        let created = sqlx::query_file_scalar!("queries/prepare_partitions.sql", through)
            .fetch_one(self.store.pool())
            .await?;
        stored("partition count", u32::try_from(created))
    }
}

fn money(currency: &str, amount: Option<String>) -> Result<Option<Money>, UsageStoreError> {
    if currency == "XXX" {
        return Ok(None);
    }
    let amount =
        amount.ok_or_else(|| UsageStoreError::Invalid("money without an amount".into()))?;
    Ok(Some(Money {
        amount: stored("amount", Amount::parse(amount))?,
        currency: stored("currency", Currency::parse(currency))?,
    }))
}

type Columns = (Vec<String>, Vec<String>, Vec<String>, Vec<bool>);

fn columns(kinds: &[UsageKindDeclaration]) -> Columns {
    let mut columns: Columns = Default::default();
    for kind in kinds {
        columns.0.push(kind.kind.as_str().to_owned());
        columns.1.push(kind.service_category.as_str().to_owned());
        columns.2.push(kind.unit.as_str().to_owned());
        columns.3.push(kind.monetary);
    }
    columns
}

/// Appends inside a caller's transaction, so usage commits or rolls back with the work
/// it meters. Each record's source is the owner of its kind. A repeated draft with the
/// same source, kind, phase and source identifier writes nothing.
pub struct UsageWrite;

impl UsageWrite {
    /// Append as the installation owner.
    pub async fn append(
        connection: &mut PgConnection,
        drafts: &[UsageDraft],
    ) -> Result<u64, UsageStoreError> {
        let rows: Vec<Value> = drafts.iter().map(UsageDraft::to_row).collect();
        let added = sqlx::query_file_scalar!("queries/append.sql", &rows)
            .fetch_one(&mut *connection)
            .await?;
        stored("added count", u64::try_from(added))
    }

    /// Append as a registered server through its own login. Every kind must be one the
    /// login's servers declared.
    pub async fn append_as_server(
        connection: &mut PgConnection,
        drafts: &[UsageDraft],
    ) -> Result<u64, UsageStoreError> {
        let rows: Vec<Value> = drafts.iter().map(UsageDraft::to_row).collect();
        let added = sqlx::query_file_scalar!("queries/append_server.sql", &rows)
            .fetch_one(&mut *connection)
            .await?;
        stored("added count", u64::try_from(added))
    }

    /// Record a server's estimate before billed work, in the transaction that commits
    /// the work's dispatch. The estimate is the work's reservation: an installation's
    /// budgets refuse it here, before the provider is called.
    pub async fn reserve(
        connection: &mut PgConnection,
        estimate: &UsageDraft,
    ) -> Result<UsageReservation, UsageStoreError> {
        if estimate.phase() != UsagePhase::Estimate {
            return Err(UsageStoreError::Refused(
                "a reservation is an estimate".into(),
            ));
        }
        Self::append_as_server(connection, std::slice::from_ref(estimate)).await?;
        Ok(UsageReservation {
            kind: estimate.kind().clone(),
            source_id: estimate.source_id().clone(),
        })
    }
}

/// An estimate the ledger accepted before billed work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageReservation {
    pub kind: UsageKind,
    pub source_id: UsageSourceId,
}
