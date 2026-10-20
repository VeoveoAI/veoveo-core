//! Outside work a Task hands to a provider and waits for.
//!
//! The owning worker records an operation before it dispatches it, so a Task that runs
//! again after recovery never sends the same work twice. While the outcome is pending,
//! the worker parks the Task: the Task waits without an owner, holds no worker and
//! survives restarts. A provider event, a due status check or the operation's deadline
//! queues the Task again, and its work replays its finished steps from the database and
//! continues where it waited.
use std::{fmt, future::Future, time::Duration};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, TimeDelta, Utc};
use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use veoveo_types::{IdMetadata, IdProfile, IdProfileSpec, IdentifierError, TaskId};

use crate::{
    InstanceId, StepDispatch, StepOperationId, TaskError, TaskRuntime, TaskStatus,
    runtime::{MAX_ATTEMPTS, validate_task_id},
    snapshot::is_valid_key,
};

mod deliver;
pub(crate) mod due;

pub use deliver::{Delivery, EventTarget, OperationEvent};
pub use due::{DUE_INTERVAL, DuePass};

/// The longest operation key, in bytes.
pub const MAX_OPERATION_KEY_BYTES: usize = 128;

/// The name a Task's work gives one of its operations: 1 to 128 bytes without control
/// characters, the same on every run.
#[veoveo_types::id(text(OperationKeys))]
pub struct OperationKey(String);

fn validate_operation_key(value: &str, _: IdMetadata) -> Result<(), IdentifierError> {
    if value.len() <= MAX_OPERATION_KEY_BYTES && is_valid_key(value) {
        Ok(())
    } else {
        Err(IdentifierError::new(
            value,
            "must be 1 to 128 bytes without control characters",
        ))
    }
}

#[doc(hidden)]
pub struct OperationKeys;
impl IdProfile for OperationKeys {
    type Error = IdentifierError;
    const PROFILE: IdProfileSpec<Self::Error> = IdProfileSpec::text(validate_operation_key);
}

/// The name a server gives the provider of one kind of outside work: a lowercase ASCII
/// letter followed by up to 63 lowercase letters, digits, dots, underscores or hyphens.
/// It scopes the provider's job and event identifiers within the server.
#[veoveo_types::id(text(ProviderNames))]
pub struct ProviderName(String);

fn validate_provider(value: &str, _: IdMetadata) -> Result<(), IdentifierError> {
    let mut chars = value.chars();
    let valid = value.len() <= 64
        && chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c));
    if valid {
        Ok(())
    } else {
        Err(IdentifierError::new(
            value,
            "must be a lowercase letter followed by up to 63 lowercase letters, digits, '.', '_' or '-'",
        ))
    }
}

#[doc(hidden)]
pub struct ProviderNames;
impl IdProfile for ProviderNames {
    type Error = IdentifierError;
    const PROFILE: IdProfileSpec<Self::Error> = IdProfileSpec::text(validate_provider);
}

/// A provider's identifier of one job: 1 to 512 bytes without control characters.
#[veoveo_types::id(text(ExternalJobIds))]
pub struct ExternalJobId(String);

fn validate_external(value: &str, _: IdMetadata) -> Result<(), IdentifierError> {
    if !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control) {
        Ok(())
    } else {
        Err(IdentifierError::new(
            value,
            "must be 1 to 512 bytes without control characters",
        ))
    }
}

#[doc(hidden)]
pub struct ExternalJobIds;
impl IdProfile for ExternalJobIds {
    type Error = IdentifierError;
    const PROFILE: IdProfileSpec<Self::Error> = IdProfileSpec::text(validate_external);
}

/// A provider's identifier of one event, such as a webhook delivery ID: 1 to 256 bytes
/// without control characters. A repeated identifier is the same event.
#[veoveo_types::id(text(OperationEventIds))]
pub struct OperationEventId(String);

fn validate_event(value: &str, _: IdMetadata) -> Result<(), IdentifierError> {
    if is_valid_key(value) {
        Ok(())
    } else {
        Err(IdentifierError::new(
            value,
            "must be 1 to 256 bytes without control characters",
        ))
    }
}

#[doc(hidden)]
pub struct OperationEventIds;
impl IdProfile for OperationEventIds {
    type Error = IdentifierError;
    const PROFILE: IdProfileSpec<Self::Error> = IdProfileSpec::text(validate_event);
}

/// The bearer secret a callback URL carries: 256 random bits as 43 base64url
/// characters. The runtime stores only its SHA-256, so the token exists only in the
/// dispatch that created it and in the provider's callback.
#[derive(Clone, PartialEq, Eq)]
pub struct CallbackToken(String);

impl CallbackToken {
    fn fresh() -> Result<Self, TaskError> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|error| {
            TaskError::invalid_record(format!("no random bytes for a callback token: {error}"))
        })?;
        Ok(Self(URL_SAFE_NO_PAD.encode(bytes)))
    }

    /// Admit a token a callback presented.
    pub fn parse(value: &str) -> Option<Self> {
        let decoded = URL_SAFE_NO_PAD.decode(value).ok()?;
        (decoded.len() == 32 && value.len() == 43).then(|| Self(value.to_owned()))
    }

    /// The token as it goes into the callback URL.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub(crate) fn digest(&self) -> Vec<u8> {
        Sha256::digest(self.0.as_bytes()).to_vec()
    }
}

impl fmt::Debug for CallbackToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CallbackToken(redacted)")
    }
}

/// When a provider's outcome reaches the runtime.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Completion {
    /// The provider reports through events, such as webhooks, that the server delivers.
    Events,
    /// The server asks the provider for the outcome on a declared schedule.
    Checks(CheckSchedule),
}

/// A bounded status-check schedule: the first check `first_after` the dispatch, the next
/// `interval` later, each later interval `backoff` times the one before, and at most
/// `limit` checks before the deadline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckSchedule {
    first_after: Duration,
    interval: Duration,
    backoff: f64,
    limit: u32,
}

impl CheckSchedule {
    /// A schedule with an interval of at least one millisecond, a backoff from 1 to 10
    /// and 1 to 10,000 checks.
    pub fn new(
        first_after: Duration,
        interval: Duration,
        backoff: f64,
        limit: u32,
    ) -> Result<Self, TaskError> {
        let valid = interval >= Duration::from_millis(1)
            && interval <= Duration::from_secs(86_400)
            && first_after <= Duration::from_secs(86_400)
            && backoff.is_finite()
            && (1.0..=10.0).contains(&backoff)
            && (1..=10_000).contains(&limit);
        if !valid {
            return Err(TaskError::invalid_record(
                "a check schedule needs an interval of 1 ms to 1 day, a first check within a day, a backoff from 1 to 10 and 1 to 10,000 checks",
            ));
        }
        Ok(Self {
            first_after,
            interval,
            backoff,
            limit,
        })
    }

    pub fn limit(&self) -> u32 {
        self.limit
    }
}

/// An operation to dispatch: its key, its provider, how its outcome arrives and the
/// deadline after which an outcome that has not arrived is unresolved.
#[derive(Clone, Debug, PartialEq)]
pub struct OperationSpec {
    pub key: OperationKey,
    pub provider: ProviderName,
    pub completion: Completion,
    pub deadline: DateTime<Utc>,
}

impl OperationSpec {
    /// An operation whose outcome arrives through events, within `timeout`.
    pub fn events(key: OperationKey, provider: ProviderName, timeout: Duration) -> Self {
        Self {
            key,
            provider,
            completion: Completion::Events,
            deadline: deadline_after(timeout),
        }
    }

    /// An operation whose outcome the server checks for on `schedule`, within `timeout`.
    pub fn checks(
        key: OperationKey,
        provider: ProviderName,
        schedule: CheckSchedule,
        timeout: Duration,
    ) -> Self {
        Self {
            key,
            provider,
            completion: Completion::Checks(schedule),
            deadline: deadline_after(timeout),
        }
    }
}

fn deadline_after(timeout: Duration) -> DateTime<Utc> {
    Utc::now() + TimeDelta::from_std(timeout).unwrap_or(TimeDelta::MAX)
}

/// What the dispatch closure learns before it calls the provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationAttempt {
    /// The operation's identity, the same on every attempt: the provider call's
    /// idempotency key.
    pub operation_id: StepOperationId,
    /// [`StepDispatch::Unsettled`] when an earlier attempt dispatched the operation and
    /// recorded no submission. Its call may have reached the provider, so this attempt
    /// reconciles with the provider and never resubmits blindly.
    pub dispatch: StepDispatch,
    /// The callback token for the provider's callback URL, on the first attempt only.
    /// A later attempt has none: the provider already holds the first one.
    pub callback: Option<CallbackToken>,
}

/// What the dispatch closure reports once the provider accepted the work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Submission {
    pub external_id: Option<ExternalJobId>,
}

impl Submission {
    /// The provider accepted the job under its own identifier.
    pub fn job(external_id: ExternalJobId) -> Self {
        Self {
            external_id: Some(external_id),
        }
    }

    /// The work was handed over, and the provider reports it by operation identity.
    pub fn handed_over() -> Self {
        Self::default()
    }
}

/// Where an operation stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationState {
    /// Recorded, and the dispatch has not recorded a submission.
    Dispatching,
    /// The provider accepted the work.
    Submitted,
    /// A final event settled the operation.
    Settled,
    /// The deadline passed without a final event. A final event can still settle it.
    Unresolved,
}

impl OperationState {
    fn parse(value: &str) -> Result<Self, TaskError> {
        Ok(match value {
            "dispatching" => Self::Dispatching,
            "submitted" => Self::Submitted,
            "settled" => Self::Settled,
            "unresolved" => Self::Unresolved,
            other => {
                return Err(TaskError::invalid_record(format!(
                    "stored operation state `{other}`"
                )));
            }
        })
    }

    fn is_pending(self) -> bool {
        matches!(self, Self::Dispatching | Self::Submitted)
    }
}

/// One operation as the database holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Operation {
    pub task_id: TaskId,
    pub key: OperationKey,
    pub operation_id: StepOperationId,
    pub provider: ProviderName,
    pub external_id: Option<ExternalJobId>,
    pub state: OperationState,
    pub completion: Completion,
    pub checks_made: u32,
    pub next_check_at: Option<DateTime<Utc>>,
    pub deadline: DateTime<Utc>,
    /// The final event's payload once the operation settled.
    pub outcome: Option<Value>,
    pub dispatched_at: DateTime<Utc>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub settled_at: Option<DateTime<Utc>>,
    pub unresolved_at: Option<DateTime<Utc>>,
    pub cancel_started_at: Option<DateTime<Utc>>,
    pub cancel_receipt: Option<Value>,
}

pub(crate) struct OperationRow {
    pub task_id: Uuid,
    pub operation_key: String,
    pub operation_id: StepOperationId,
    pub provider: String,
    pub external_id: Option<String>,
    pub state: String,
    pub completion: String,
    pub check_first_ms: Option<i64>,
    pub check_interval_ms: Option<i64>,
    pub check_backoff: Option<f64>,
    pub check_limit: Option<i32>,
    pub checks_made: i32,
    pub next_check_at: Option<DateTime<Utc>>,
    pub deadline: DateTime<Utc>,
    pub outcome: Option<Value>,
    pub dispatched_at: DateTime<Utc>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub settled_at: Option<DateTime<Utc>>,
    pub unresolved_at: Option<DateTime<Utc>>,
    pub cancel_started_at: Option<DateTime<Utc>>,
    pub cancel_receipt: Option<Value>,
}

/// A dispatch row with whether this statement recorded it.
pub(crate) struct DispatchRow {
    pub task_id: Uuid,
    pub operation_key: String,
    pub operation_id: StepOperationId,
    pub provider: String,
    pub external_id: Option<String>,
    pub state: String,
    pub completion: String,
    pub check_first_ms: Option<i64>,
    pub check_interval_ms: Option<i64>,
    pub check_backoff: Option<f64>,
    pub check_limit: Option<i32>,
    pub checks_made: i32,
    pub next_check_at: Option<DateTime<Utc>>,
    pub deadline: DateTime<Utc>,
    pub outcome: Option<Value>,
    pub dispatched_at: DateTime<Utc>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub settled_at: Option<DateTime<Utc>>,
    pub unresolved_at: Option<DateTime<Utc>>,
    pub cancel_started_at: Option<DateTime<Utc>>,
    pub cancel_receipt: Option<Value>,
    pub first: bool,
}

impl DispatchRow {
    fn split(self) -> (OperationRow, bool) {
        (
            OperationRow {
                task_id: self.task_id,
                operation_key: self.operation_key,
                operation_id: self.operation_id,
                provider: self.provider,
                external_id: self.external_id,
                state: self.state,
                completion: self.completion,
                check_first_ms: self.check_first_ms,
                check_interval_ms: self.check_interval_ms,
                check_backoff: self.check_backoff,
                check_limit: self.check_limit,
                checks_made: self.checks_made,
                next_check_at: self.next_check_at,
                deadline: self.deadline,
                outcome: self.outcome,
                dispatched_at: self.dispatched_at,
                submitted_at: self.submitted_at,
                settled_at: self.settled_at,
                unresolved_at: self.unresolved_at,
                cancel_started_at: self.cancel_started_at,
                cancel_receipt: self.cancel_receipt,
            },
            self.first,
        )
    }
}

fn stored<T: std::str::FromStr>(field: &str, value: &str) -> Result<T, TaskError>
where
    T::Err: fmt::Display,
{
    value
        .parse()
        .map_err(|error| TaskError::invalid_record(format!("stored operation {field}: {error}")))
}

fn millis(field: &str, value: Option<i64>) -> Result<Duration, TaskError> {
    value
        .and_then(|value| u64::try_from(value).ok())
        .map(Duration::from_millis)
        .ok_or_else(|| TaskError::invalid_record(format!("stored operation {field}")))
}

impl TryFrom<OperationRow> for Operation {
    type Error = TaskError;

    fn try_from(row: OperationRow) -> Result<Self, Self::Error> {
        let completion = match row.completion.as_str() {
            "events" => Completion::Events,
            "checks" => Completion::Checks(CheckSchedule::new(
                millis("first check", row.check_first_ms)?,
                millis("check interval", row.check_interval_ms)?,
                row.check_backoff
                    .ok_or_else(|| TaskError::invalid_record("stored operation backoff"))?,
                row.check_limit
                    .and_then(|limit| u32::try_from(limit).ok())
                    .ok_or_else(|| TaskError::invalid_record("stored operation check limit"))?,
            )?),
            other => {
                return Err(TaskError::invalid_record(format!(
                    "stored operation completion `{other}`"
                )));
            }
        };
        Ok(Self {
            task_id: TaskId::from_uuid(row.task_id),
            key: stored("key", &row.operation_key)?,
            operation_id: row.operation_id,
            provider: stored("provider", &row.provider)?,
            external_id: row
                .external_id
                .as_deref()
                .map(|external| stored("job identifier", external))
                .transpose()?,
            state: OperationState::parse(&row.state)?,
            completion,
            checks_made: u32::try_from(row.checks_made)
                .map_err(|_| TaskError::invalid_record("stored operation check count"))?,
            next_check_at: row.next_check_at,
            deadline: row.deadline,
            outcome: row.outcome,
            dispatched_at: row.dispatched_at,
            submitted_at: row.submitted_at,
            settled_at: row.settled_at,
            unresolved_at: row.unresolved_at,
            cancel_started_at: row.cancel_started_at,
            cancel_receipt: row.cancel_receipt,
        })
    }
}

/// How a wait on an operation ended.
#[derive(Clone, Debug, PartialEq)]
pub enum WaitOutcome<T> {
    /// A final event settled the operation with this payload.
    Settled(T),
    /// The deadline passed without a final event. The outcome is unknown, which never
    /// proves failure: the operation keeps its Task until a later event settles it.
    Expired,
    /// The Task's cancellation reached the provider, which answered with this receipt.
    /// The worker settles the Task as cancelled.
    Cancelled { receipt: Value },
}

/// One status check's result.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckObservation {
    /// The provider's identifier of this observation, when it has one.
    pub event_id: Option<OperationEventId>,
    pub payload: Value,
    /// Whether the payload is the operation's outcome.
    pub is_final: bool,
}

/// The provider calls a wait makes on the server's behalf.
pub trait OperationCalls: Sync {
    type Error: From<TaskError>;

    /// Ask the provider where a checked operation stands. Only operations dispatched
    /// with [`Completion::Checks`] are checked.
    fn check(
        &self,
        operation: &Operation,
    ) -> impl Future<Output = Result<CheckObservation, Self::Error>> + Send;

    /// Carry the Task's cancellation to the provider and return its answer. A process
    /// that ends during the call leaves the cancellation started without a receipt, so
    /// the next attempt calls again.
    fn cancel(
        &self,
        operation: &Operation,
    ) -> impl Future<Output = Result<Value, Self::Error>> + Send;
}

impl TaskRuntime {
    /// Record an operation of a Task this instance owns and dispatch it once.
    ///
    /// The runtime records the operation before `run` starts. `run` receives the
    /// operation's identity, whether an earlier attempt already dispatched it, and on
    /// the first attempt the callback token. A successful `run` records its submission
    /// and every later call returns the operation without running `run`. An error from
    /// `run` records nothing and returns as it is, leaving the next attempt unsettled.
    pub async fn dispatch_operation<E, F, Fut>(
        &self,
        task_id: TaskId,
        spec: &OperationSpec,
        run: F,
    ) -> Result<Operation, E>
    where
        E: From<TaskError>,
        F: FnOnce(OperationAttempt) -> Fut,
        Fut: Future<Output = Result<Submission, E>>,
    {
        let task_id = validate_task_id(task_id)?;
        let callback = CallbackToken::fresh()?;
        let (schedule_first, schedule_interval, backoff, limit) = match spec.completion {
            Completion::Events => (None, None, None, None),
            Completion::Checks(schedule) => (
                Some(duration_ms(schedule.first_after)),
                Some(duration_ms(schedule.interval)),
                Some(schedule.backoff),
                Some(i32::try_from(schedule.limit).unwrap_or(i32::MAX)),
            ),
        };
        let row = sqlx::query_file_as!(
            DispatchRow,
            "queries/operation_dispatch.sql",
            task_id.as_uuid(),
            self.server().as_str(),
            self.instance_id() as InstanceId,
            spec.key.as_str(),
            StepOperationId::new() as StepOperationId,
            spec.provider.as_str(),
            completion_name(&spec.completion),
            schedule_first,
            schedule_interval,
            backoff,
            limit,
            spec.deadline,
            callback.digest(),
            self.deployment().as_uuid(),
        )
        .fetch_optional(self.pool())
        .await
        .map_err(TaskError::from)?;
        let Some(row) = row else {
            return Err(self.rejected(task_id).await.into());
        };
        let (row, first) = row.split();
        let operation = Operation::try_from(row)?;
        if operation.provider != spec.provider || operation.completion != spec.completion {
            return Err(TaskError::invalid_record(format!(
                "operation `{}` was recorded with another provider or completion",
                spec.key
            ))
            .into());
        }
        if operation.state != OperationState::Dispatching {
            return Ok(operation);
        }
        let attempt = OperationAttempt {
            operation_id: operation.operation_id,
            dispatch: if first {
                StepDispatch::First
            } else {
                StepDispatch::Unsettled {
                    dispatched_at: operation.dispatched_at,
                }
            },
            callback: first.then_some(callback),
        };
        let submission = run(attempt).await?;
        let row = sqlx::query_file_as!(
            OperationRow,
            "queries/operation_submit.sql",
            task_id.as_uuid(),
            self.server().as_str(),
            self.instance_id() as InstanceId,
            spec.key.as_str(),
            operation.operation_id as StepOperationId,
            submission.external_id.as_ref().map(ExternalJobId::as_str),
            self.deployment().as_uuid(),
        )
        .fetch_optional(self.pool())
        .await
        .map_err(TaskError::from)?;
        match row {
            Some(row) => Ok(Operation::try_from(row)?),
            // A delivery may have settled the operation before its submission was
            // recorded; the stored operation is current.
            None => match self.operation(task_id, &spec.key).await? {
                Some(current) if current.state != OperationState::Dispatching => Ok(current),
                _ => Err(self.rejected(task_id).await.into()),
            },
        }
    }

    /// One operation of this server's Task.
    pub async fn operation(
        &self,
        task_id: TaskId,
        key: &OperationKey,
    ) -> Result<Option<Operation>, TaskError> {
        let task_id = validate_task_id(task_id)?;
        sqlx::query_file_as!(
            OperationRow,
            "queries/operation_get.sql",
            task_id.as_uuid(),
            key.as_str(),
            self.server().as_str(),
            self.deployment().as_uuid(),
        )
        .fetch_optional(self.pool())
        .await?
        .map(Operation::try_from)
        .transpose()
    }

    /// Wait for an operation of a Task this instance owns.
    ///
    /// A settled operation returns its outcome and an unresolved one
    /// [`WaitOutcome::Expired`]. A checked operation whose check is due runs `calls`'
    /// check and records the observation. A Task whose cancellation was requested
    /// carries it to the provider through `calls`' cancel and returns its receipt.
    /// Otherwise the worker parks the Task and the wait fails with
    /// [`TaskError::Parked`]: the worker propagates it and stops, and the Task runs
    /// again once an event, a due check or the deadline queues it.
    pub async fn wait_operation<T, C>(
        &self,
        task_id: TaskId,
        key: &OperationKey,
        calls: &C,
    ) -> Result<WaitOutcome<T>, C::Error>
    where
        T: DeserializeOwned,
        C: OperationCalls,
    {
        let task_id = validate_task_id(task_id)?;
        for _ in 0..MAX_ATTEMPTS {
            let operation = self
                .operation(task_id, key)
                .await?
                .ok_or_else(|| TaskError::invalid_record(format!("no operation `{key}`")))?;
            match operation.state {
                OperationState::Settled => {
                    let outcome = operation.outcome.ok_or_else(|| {
                        TaskError::invalid_record("a settled operation has no outcome")
                    })?;
                    return Ok(WaitOutcome::Settled(
                        serde_json::from_value(outcome).map_err(TaskError::from)?,
                    ));
                }
                OperationState::Unresolved => return Ok(WaitOutcome::Expired),
                OperationState::Dispatching | OperationState::Submitted => {}
            }
            let task = self
                .get(task_id)
                .await?
                .ok_or(TaskError::NotFound(task_id))?;
            if task.owner_instance != Some(self.instance_id()) {
                return Err(TaskError::NotOwner(task_id).into());
            }
            if task.status == TaskStatus::CancelRequested {
                return self.carry_cancellation(&operation, calls).await;
            }
            if let Completion::Checks(_) = operation.completion
                && operation.next_check_at.is_some_and(|due| due <= Utc::now())
            {
                self.record_check(&operation, calls).await?;
                continue;
            }
            if self.park(&operation).await? {
                return Err(TaskError::Parked(task_id).into());
            }
        }
        Err(TaskError::Conflict(task_id).into())
    }

    async fn record_check<C: OperationCalls>(
        &self,
        operation: &Operation,
        calls: &C,
    ) -> Result<(), C::Error> {
        let observation = calls.check(operation).await?;
        let event_id = match observation.event_id {
            Some(event_id) => event_id,
            None => OperationEventId::parse(format!(
                "check:{}:{}",
                operation.operation_id,
                operation.checks_made + 1
            ))
            .map_err(|error| TaskError::invalid_record(error.to_string()))?,
        };
        sqlx::query_file_as!(
            OperationRow,
            "queries/operation_check.sql",
            operation.task_id.as_uuid(),
            self.server().as_str(),
            self.instance_id() as InstanceId,
            operation.key.as_str(),
            event_id.as_str(),
            observation.payload,
            observation.is_final,
            self.deployment().as_uuid(),
        )
        .fetch_optional(self.pool())
        .await
        .map_err(TaskError::from)?;
        Ok(())
    }

    /// Park the Task on `operation`. False when the operation is no longer pending or
    /// a check fell due, so the wait reads it again.
    async fn park(&self, operation: &Operation) -> Result<bool, TaskError> {
        let parked = sqlx::query_file_scalar!(
            "queries/operation_park.sql",
            operation.task_id.as_uuid(),
            self.server().as_str(),
            self.instance_id() as InstanceId,
            operation.key.as_str(),
            format!("Waiting for {}", operation.provider),
            self.deployment().as_uuid(),
        )
        .fetch_optional(self.pool())
        .await?;
        if parked.is_some() {
            return Ok(true);
        }
        let current = self
            .operation(operation.task_id, &operation.key)
            .await?
            .ok_or_else(|| {
                TaskError::invalid_record(format!("no operation `{}`", operation.key))
            })?;
        if current.state.is_pending() && current.next_check_at.is_none_or(|due| due > Utc::now()) {
            // The operation is still pending, so the Task's ownership or status changed.
            let task = self
                .get(operation.task_id)
                .await?
                .ok_or(TaskError::NotFound(operation.task_id))?;
            if task.owner_instance != Some(self.instance_id()) {
                return Err(TaskError::NotOwner(operation.task_id));
            }
        }
        Ok(false)
    }

    async fn carry_cancellation<T, C: OperationCalls>(
        &self,
        operation: &Operation,
        calls: &C,
    ) -> Result<WaitOutcome<T>, C::Error> {
        if let Some(receipt) = &operation.cancel_receipt {
            return Ok(WaitOutcome::Cancelled {
                receipt: receipt.clone(),
            });
        }
        let started = sqlx::query_file_scalar!(
            "queries/operation_cancel_start.sql",
            operation.task_id.as_uuid(),
            self.server().as_str(),
            self.instance_id() as InstanceId,
            operation.key.as_str(),
            self.deployment().as_uuid(),
        )
        .fetch_optional(self.pool())
        .await
        .map_err(TaskError::from)?;
        if started.is_none() {
            return Err(self.rejected(operation.task_id).await.into());
        }
        let receipt = calls.cancel(operation).await?;
        let recorded = sqlx::query_file_scalar!(
            "queries/operation_cancel_record.sql",
            operation.task_id.as_uuid(),
            self.server().as_str(),
            self.instance_id() as InstanceId,
            operation.key.as_str(),
            receipt,
            self.deployment().as_uuid(),
        )
        .fetch_optional(self.pool())
        .await
        .map_err(TaskError::from)?;
        match recorded {
            Some(receipt) => Ok(WaitOutcome::Cancelled { receipt }),
            None => Err(self.rejected(operation.task_id).await.into()),
        }
    }
}

fn duration_ms(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

fn completion_name(completion: &Completion) -> &'static str {
    match completion {
        Completion::Events => "events",
        Completion::Checks(_) => "checks",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_their_grammar() {
        assert!(OperationKey::parse("generate").is_ok());
        assert!(OperationKey::parse("k".repeat(129)).is_err());
        assert!(OperationKey::parse("").is_err());
        assert!(ProviderName::parse("media.images").is_ok());
        assert!(ProviderName::parse("Media").is_err());
        assert!(ProviderName::parse("1media").is_err());
        assert!(ProviderName::parse("m".repeat(65)).is_err());
        assert!(ExternalJobId::parse("job/42 a").is_ok());
        assert!(ExternalJobId::parse("j".repeat(513)).is_err());
        assert!(OperationEventId::parse("evt\n1").is_err());
    }

    #[test]
    fn callback_tokens_are_256_random_bits_and_never_print() {
        let token = CallbackToken::fresh().unwrap();
        assert_eq!(token.expose().len(), 43);
        assert_eq!(CallbackToken::parse(token.expose()), Some(token.clone()));
        assert_ne!(CallbackToken::fresh().unwrap(), token);
        assert!(CallbackToken::parse("short").is_none());
        assert!(!format!("{token:?}").contains(token.expose()));
        assert_eq!(token.digest().len(), 32);
    }

    #[test]
    fn check_schedules_are_bounded() {
        let second = Duration::from_secs(1);
        assert!(CheckSchedule::new(second, second, 2.0, 10).is_ok());
        assert!(CheckSchedule::new(second, Duration::ZERO, 2.0, 10).is_err());
        assert!(CheckSchedule::new(second, second, 0.5, 10).is_err());
        assert!(CheckSchedule::new(second, second, f64::NAN, 10).is_err());
        assert!(CheckSchedule::new(second, second, 2.0, 0).is_err());
        assert!(CheckSchedule::new(second, second, 2.0, 10_001).is_err());
    }
}
