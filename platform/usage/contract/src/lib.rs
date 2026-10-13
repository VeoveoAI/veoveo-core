//! Typed usage records: what a server metered, for whom, in the columns of the FinOps
//! Open Cost and Usage Specification (FOCUS) 1.4. Each server declares its own kinds of
//! usage; this crate names no server, unit vocabulary beyond FOCUS's format, or price.
use std::str::FromStr;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;
use veoveo_types::{
    IdMetadata, IdProfile, IdProfileSpec, IdentifierError, PrincipalId, ServerSlug, TaskId,
    TenantId, WorkContextId,
};

/// Why a usage value or draft is not admitted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid usage {field}: {reason}")]
pub struct UsageError {
    pub field: &'static str,
    pub reason: &'static str,
}

fn invalid(field: &'static str, reason: &'static str) -> UsageError {
    UsageError { field, reason }
}

fn is_owner(owner: &str) -> bool {
    let mut chars = owner.chars();
    owner.len() <= 48
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 63
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A kind of usage, `<owner>.<name>`: the owner is the declaring server's slug and the
/// name a snake_case noun, such as `media.generation` or `reason.output_tokens`.
#[veoveo_types::id(text(UsageKinds))]
pub struct UsageKind(String);

fn validate_kind(value: &str, _: IdMetadata) -> Result<(), IdentifierError> {
    if value
        .split_once('.')
        .is_some_and(|(owner, name)| is_owner(owner) && is_name(name))
    {
        Ok(())
    } else {
        Err(IdentifierError::new(
            value,
            "must be <owner>.<snake_case name>",
        ))
    }
}

#[doc(hidden)]
pub struct UsageKinds;
impl IdProfile for UsageKinds {
    type Error = IdentifierError;
    const PROFILE: IdProfileSpec<Self::Error> = IdProfileSpec::text(validate_kind);
}

impl UsageKind {
    /// The server that owns the kind and is the source of its records.
    pub fn owner(&self) -> &str {
        self.as_str().split_once('.').map_or("", |(owner, _)| owner)
    }
}

/// The producer's identifier of what a record counts, unique within its source, kind
/// and phase, such as `task:<id>:output:2`. Repeating a write with the same identifier
/// writes nothing.
#[veoveo_types::id(text(UsageSourceIds))]
pub struct UsageSourceId(String);

fn validate_source(value: &str, _: IdMetadata) -> Result<(), IdentifierError> {
    if !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control) {
        Ok(())
    } else {
        Err(IdentifierError::new(
            value,
            "must be 1 to 256 bytes without control characters",
        ))
    }
}

#[doc(hidden)]
pub struct UsageSourceIds;
impl IdProfile for UsageSourceIds {
    type Error = IdentifierError;
    const PROFILE: IdProfileSpec<Self::Error> = IdProfileSpec::text(validate_source);
}

/// Admit `value` for a checked text value.
fn checked(
    value: String,
    rule: fn(&str) -> bool,
    field: &'static str,
    reason: &'static str,
) -> Result<String, UsageError> {
    if rule(&value) {
        Ok(value)
    } else {
        Err(invalid(field, reason))
    }
}

fn is_unit(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " /-".contains(c))
}

/// A FOCUS 1.4 Unit Format value, such as `Tokens`, `Requests`, `Hours` or `GB-Hours`:
/// 1 to 64 ASCII letters, digits, spaces, hyphens and slashes, starting with a letter or
/// digit.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct UsageUnit(String);

impl UsageUnit {
    pub fn parse(value: impl Into<String>) -> Result<Self, UsageError> {
        checked(
            value.into(),
            is_unit,
            "unit",
            "must be a FOCUS unit of 1 to 64 letters, digits, spaces, hyphens or slashes",
        )
        .map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn tokens() -> Self {
        Self("Tokens".into())
    }

    pub fn requests() -> Self {
        Self("Requests".into())
    }

    pub fn units() -> Self {
        Self("Units".into())
    }

    pub fn seconds() -> Self {
        Self("Seconds".into())
    }

    pub fn hours() -> Self {
        Self("Hours".into())
    }

    pub fn gigabytes() -> Self {
        Self("GB".into())
    }

    pub fn gigabyte_hours() -> Self {
        Self("GB-Hours".into())
    }
}

impl<'de> Deserialize<'de> for UsageUnit {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

fn is_currency(value: &str) -> bool {
    value.len() == 3 && value.chars().all(|c| c.is_ascii_uppercase()) && value != "XXX"
}

/// An ISO 4217 alphabetic currency code, such as `USD`. `XXX`, which ISO 4217 reserves
/// for no currency, marks a record without money in storage and is not a `Currency`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct Currency(String);

impl Currency {
    pub fn parse(value: impl Into<String>) -> Result<Self, UsageError> {
        checked(
            value.into(),
            is_currency,
            "currency",
            "must be an ISO 4217 alphabetic code other than XXX",
        )
        .map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Currency {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

fn is_decimal(value: &str, signed: bool) -> bool {
    let digits = value.strip_prefix('-').filter(|_| signed).unwrap_or(value);
    let (whole, fraction) = match digits.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (digits, None),
    };
    let whole_ok = !whole.is_empty()
        && whole.len() <= 30
        && whole.chars().all(|c| c.is_ascii_digit())
        && (whole == "0" || !whole.starts_with('0'));
    let fraction_ok = fraction.is_none_or(|fraction| {
        !fraction.is_empty() && fraction.len() <= 18 && fraction.chars().all(|c| c.is_ascii_digit())
    });
    whole_ok && fraction_ok && value != "-0"
}

fn is_quantity(value: &str) -> bool {
    is_decimal(value, false)
}

fn is_amount(value: &str) -> bool {
    is_decimal(value, true)
}

/// A non-negative exact decimal with at most 30 integer and 18 fractional digits,
/// written as text so no value passes through a binary float.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct Quantity(String);

impl Quantity {
    pub fn parse(value: impl Into<String>) -> Result<Self, UsageError> {
        checked(
            value.into(),
            is_quantity,
            "quantity",
            "must be a non-negative decimal with at most 30 integer and 18 fractional digits",
        )
        .map(Self)
    }

    pub fn whole(value: u64) -> Self {
        Self(value.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Quantity {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// An exact decimal amount of money, negative for a credit, with at most 30 integer and
/// 18 fractional digits.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct Amount(String);

impl Amount {
    pub fn parse(value: impl Into<String>) -> Result<Self, UsageError> {
        checked(
            value.into(),
            is_amount,
            "amount",
            "must be a decimal with at most 30 integer and 18 fractional digits",
        )
        .map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Amount {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// An amount in one currency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Money {
    pub amount: Amount,
    pub currency: Currency,
}

/// FOCUS 1.4 ServiceCategory.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum ServiceCategory {
    #[serde(rename = "AI and Machine Learning")]
    AiAndMachineLearning,
    Analytics,
    #[serde(rename = "Business Applications")]
    BusinessApplications,
    Compute,
    Databases,
    #[serde(rename = "Developer Tools")]
    DeveloperTools,
    Multicloud,
    Identity,
    Integration,
    #[serde(rename = "Internet of Things")]
    InternetOfThings,
    #[serde(rename = "Management and Governance")]
    ManagementAndGovernance,
    Media,
    Migration,
    Mobile,
    Networking,
    Security,
    Storage,
    Web,
    Other,
}

impl ServiceCategory {
    pub const ALL: &'static [Self] = &[
        Self::AiAndMachineLearning,
        Self::Analytics,
        Self::BusinessApplications,
        Self::Compute,
        Self::Databases,
        Self::DeveloperTools,
        Self::Multicloud,
        Self::Identity,
        Self::Integration,
        Self::InternetOfThings,
        Self::ManagementAndGovernance,
        Self::Media,
        Self::Migration,
        Self::Mobile,
        Self::Networking,
        Self::Security,
        Self::Storage,
        Self::Web,
        Self::Other,
    ];

    /// The FOCUS spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AiAndMachineLearning => "AI and Machine Learning",
            Self::Analytics => "Analytics",
            Self::BusinessApplications => "Business Applications",
            Self::Compute => "Compute",
            Self::Databases => "Databases",
            Self::DeveloperTools => "Developer Tools",
            Self::Multicloud => "Multicloud",
            Self::Identity => "Identity",
            Self::Integration => "Integration",
            Self::InternetOfThings => "Internet of Things",
            Self::ManagementAndGovernance => "Management and Governance",
            Self::Media => "Media",
            Self::Migration => "Migration",
            Self::Mobile => "Mobile",
            Self::Networking => "Networking",
            Self::Security => "Security",
            Self::Storage => "Storage",
            Self::Web => "Web",
            Self::Other => "Other",
        }
    }
}

impl FromStr for ServiceCategory {
    type Err = UsageError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|category| category.as_str() == value)
            .ok_or_else(|| invalid("service category", "must be a FOCUS ServiceCategory value"))
    }
}

/// Whether a record estimates usage before the work or reports what the provider
/// billed. A Task's cost is its actual records of a kind when it has any, and its
/// estimates otherwise.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum UsagePhase {
    Estimate,
    Actual,
}

impl UsagePhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Estimate => "estimate",
            Self::Actual => "actual",
        }
    }
}

impl FromStr for UsagePhase {
    type Err = UsageError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "estimate" => Ok(Self::Estimate),
            "actual" => Ok(Self::Actual),
            _ => Err(invalid("phase", "must be estimate or actual")),
        }
    }
}

/// FOCUS 1.4 ChargeCategory, limited to what a server meters: usage, a credit such as a
/// refund, and an adjustment.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum ChargeCategory {
    Usage,
    Credit,
    Adjustment,
}

impl ChargeCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usage => "Usage",
            Self::Credit => "Credit",
            Self::Adjustment => "Adjustment",
        }
    }
}

impl FromStr for ChargeCategory {
    type Err = UsageError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "Usage" => Ok(Self::Usage),
            "Credit" => Ok(Self::Credit),
            "Adjustment" => Ok(Self::Adjustment),
            _ => Err(invalid(
                "charge category",
                "must be Usage, Credit or Adjustment",
            )),
        }
    }
}

/// A kind a server declares when it starts: what its records count and whether they
/// carry money.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UsageKindDeclaration {
    pub kind: UsageKind,
    pub service_category: ServiceCategory,
    pub unit: UsageUnit,
    pub monetary: bool,
}

/// Whom a record's usage is for: the caller the work ran for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UsageActor {
    pub tenant: TenantId,
    pub principal: PrincipalId,
    pub work_context: WorkContextId,
}

/// Another server that did the work for the caller through the Gateway, and its Task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UsageVia {
    pub server: ServerSlug,
    pub task: Option<TaskId>,
}

/// When the metered usage happened. FOCUS ChargePeriodStart is inclusive and
/// ChargePeriodEnd exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChargePeriod {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl ChargePeriod {
    pub fn new(start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Self, UsageError> {
        if start < end {
            Ok(Self { start, end })
        } else {
            Err(invalid("charge period", "must start before it ends"))
        }
    }

    /// A period of one microsecond at `at`, for usage that happened at once.
    pub fn at(at: DateTime<Utc>) -> Self {
        Self {
            start: at,
            end: at + chrono::TimeDelta::microseconds(1),
        }
    }
}

/// One usage record to append. Build it with [`UsageDraft::builder`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UsageDraft {
    id: Uuid,
    kind: UsageKind,
    phase: UsagePhase,
    actor: UsageActor,
    via: Option<UsageVia>,
    task: Option<TaskId>,
    operation_key: Option<String>,
    charge_category: ChargeCategory,
    correction: bool,
    quantity: Quantity,
    money: Option<Money>,
    target: Option<String>,
    source_id: UsageSourceId,
    period: ChargePeriod,
}

impl UsageDraft {
    pub fn builder(
        kind: UsageKind,
        phase: UsagePhase,
        actor: UsageActor,
        quantity: Quantity,
        source_id: UsageSourceId,
        period: ChargePeriod,
    ) -> UsageDraftBuilder {
        UsageDraftBuilder(Self {
            id: Uuid::now_v7(),
            kind,
            phase,
            actor,
            via: None,
            task: None,
            operation_key: None,
            charge_category: ChargeCategory::Usage,
            correction: false,
            quantity,
            money: None,
            target: None,
            source_id,
            period,
        })
    }

    pub fn kind(&self) -> &UsageKind {
        &self.kind
    }

    pub fn phase(&self) -> UsagePhase {
        self.phase
    }

    pub fn source_id(&self) -> &UsageSourceId {
        &self.source_id
    }

    /// The draft as the database's append functions read it.
    pub fn to_row(&self) -> Value {
        let mut row = Map::new();
        let mut put = |key: &str, value: Value| {
            row.insert(key.to_owned(), value);
        };
        put("id", Value::String(self.id.to_string()));
        put(
            "tenant",
            Value::String(self.actor.tenant.as_str().to_owned()),
        );
        put(
            "principal",
            Value::String(self.actor.principal.as_str().to_owned()),
        );
        put(
            "work_context",
            Value::String(self.actor.work_context.as_str().to_owned()),
        );
        put("kind", Value::String(self.kind.as_str().to_owned()));
        if let Some(via) = &self.via {
            put(
                "acting_server",
                Value::String(via.server.as_str().to_owned()),
            );
            if let Some(task) = via.task {
                put("acting_task", Value::String(task.as_uuid().to_string()));
            }
        }
        if let Some(task) = self.task {
            put("task_id", Value::String(task.as_uuid().to_string()));
        }
        if let Some(key) = &self.operation_key {
            put("operation_key", Value::String(key.clone()));
        }
        put("phase", Value::String(self.phase.as_str().to_owned()));
        put(
            "charge_category",
            Value::String(self.charge_category.as_str().to_owned()),
        );
        put("correction", Value::Bool(self.correction));
        put("quantity", Value::String(self.quantity.as_str().to_owned()));
        match &self.money {
            Some(money) => {
                put("amount", Value::String(money.amount.as_str().to_owned()));
                put(
                    "currency",
                    Value::String(money.currency.as_str().to_owned()),
                );
            }
            None => put("currency", Value::String("XXX".to_owned())),
        }
        if let Some(target) = &self.target {
            put("target", Value::String(target.clone()));
        }
        put(
            "source_id",
            Value::String(self.source_id.as_str().to_owned()),
        );
        put(
            "charge_period_start",
            Value::String(self.period.start.to_rfc3339()),
        );
        put(
            "charge_period_end",
            Value::String(self.period.end.to_rfc3339()),
        );
        Value::Object(row)
    }
}

/// Optional parts of a usage draft.
#[derive(Clone, Debug)]
pub struct UsageDraftBuilder(UsageDraft);

impl UsageDraftBuilder {
    /// The server that did the work for the caller, and its Task.
    pub fn via(mut self, server: ServerSlug, task: Option<TaskId>) -> Self {
        self.0.via = Some(UsageVia { server, task });
        self
    }

    /// The Task the usage belongs to, and the operation within it.
    pub fn task(mut self, task: TaskId, operation_key: Option<&str>) -> Self {
        self.0.task = Some(task);
        self.0.operation_key = operation_key.map(str::to_owned);
        self
    }

    pub fn money(mut self, money: Money) -> Self {
        self.0.money = Some(money);
        self
    }

    pub fn charge_category(mut self, category: ChargeCategory) -> Self {
        self.0.charge_category = category;
        self
    }

    /// The record corrects an earlier one (FOCUS ChargeClass `Correction`).
    pub fn correction(mut self) -> Self {
        self.0.correction = true;
        self
    }

    /// The resource the usage is about, as its owner's URI.
    pub fn target(mut self, uri: impl Into<String>) -> Self {
        self.0.target = Some(uri.into());
        self
    }

    pub fn build(self) -> Result<UsageDraft, UsageError> {
        let draft = self.0;
        if draft.operation_key.as_deref().is_some_and(|key| {
            key.is_empty() || key.len() > 128 || key.chars().any(char::is_control)
        }) {
            return Err(invalid(
                "operation key",
                "must be 1 to 128 bytes without control characters",
            ));
        }
        if draft
            .target
            .as_deref()
            .is_some_and(|target| target.is_empty() || target.len() > 2048)
        {
            return Err(invalid("target", "must be a URI of 1 to 2048 bytes"));
        }
        if draft.kind.owner().is_empty() {
            return Err(invalid("kind", "names no owner"));
        }
        Ok(draft)
    }
}

/// One kind's cost of a Task: its actual records when it has any, else its estimates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskUsage {
    pub kind: UsageKind,
    pub unit: UsageUnit,
    pub phase: UsagePhase,
    pub quantity: Quantity,
    pub money: Option<Money>,
}

/// One day's totals of one kind for one principal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DailyUsage {
    pub day: chrono::NaiveDate,
    pub principal: PrincipalId,
    pub kind: UsageKind,
    pub phase: UsagePhase,
    pub quantity: Quantity,
    pub money: Option<Money>,
    pub records: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_follow_focus_and_iso_4217() {
        assert_eq!(
            UsageKind::parse("media.generation").unwrap().owner(),
            "media"
        );
        assert!(UsageKind::parse("media").is_err());
        assert!(UsageKind::parse("Media.generation").is_err());
        assert!(UsageUnit::parse("GB-Hours").is_ok());
        assert!(UsageUnit::parse("Units/3 Months").is_ok());
        assert!(UsageUnit::parse("-x").is_err());
        assert!(Currency::parse("USD").is_ok());
        assert!(Currency::parse("usd").is_err());
        assert!(Currency::parse("XXX").is_err());
        assert!(Quantity::parse("0.000000000000000001").is_ok());
        assert!(Quantity::parse("-1").is_err());
        assert!(Quantity::parse("01").is_err());
        assert!(Quantity::parse("1.").is_err());
        assert!(Amount::parse("-0.04").is_ok());
        assert!(Amount::parse("-0").is_err());
        assert_eq!(
            "AI and Machine Learning"
                .parse::<ServiceCategory>()
                .unwrap(),
            ServiceCategory::AiAndMachineLearning
        );
        assert_eq!(
            serde_json::to_value(ServiceCategory::InternetOfThings).unwrap(),
            serde_json::json!("Internet of Things")
        );
        for category in ServiceCategory::ALL {
            assert_eq!(
                category.as_str().parse::<ServiceCategory>().unwrap(),
                *category
            );
        }
    }
}
