//! Provider events delivered to operations: webhook deliveries a server received and
//! authenticated. Any replica stores them; the worker that resumes the Task reads them.
use serde_json::Value;

use super::{CallbackToken, ExternalJobId, OperationEventId, ProviderName};
use crate::{StepOperationId, TaskError, TaskRuntime};

/// Which operation an event is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventTarget {
    /// An event that arrived on the operation's callback URL. The token must match the
    /// one the dispatch handed to the provider.
    Callback {
        operation_id: StepOperationId,
        token: CallbackToken,
    },
    /// An event that names the operation's identity, which the server already
    /// authenticated, such as a signed payload carrying the idempotency key.
    Operation(StepOperationId),
    /// An event that names the provider's own job identifier.
    Job {
        provider: ProviderName,
        external_id: ExternalJobId,
    },
}

/// One event a provider reported, after the server checked its signature.
#[derive(Clone, Debug, PartialEq)]
pub struct OperationEvent {
    pub target: EventTarget,
    pub event_id: OperationEventId,
    pub payload: Value,
    /// Whether the payload is the operation's outcome.
    pub is_final: bool,
}

/// What a delivery did. Every outcome except [`Delivery::Unknown`] is safe to
/// acknowledge, because the event is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// A new event. A final one settled its operation and queued a Task parked on it.
    New,
    /// The event identifier was already stored; nothing changed.
    Repeat,
    /// A final event that arrived after the operation settled or its Task finished. It
    /// is stored, and a Task that finished keeps its outcome.
    Late,
    /// No operation of this server matches the target, or the callback token is wrong.
    Unknown,
}

struct DeliveryRow {
    found: i64,
    stored: i64,
    settled: i64,
    settled_task: Option<uuid::Uuid>,
    settled_key: Option<String>,
    finished: Option<bool>,
}

impl TaskRuntime {
    /// Store one provider event in one transaction. A final event settles its pending or
    /// unresolved operation, the first such event wins, and the Task parked on it is
    /// queued, so its worker resumes on any replica. The server acknowledges the
    /// provider only after this returns.
    pub async fn deliver(&self, event: &OperationEvent) -> Result<Delivery, TaskError> {
        let mut transaction = self.pool().begin().await?;
        let row = match &event.target {
            EventTarget::Callback {
                operation_id,
                token,
            } => {
                self.deliver_by_operation(
                    &mut transaction,
                    *operation_id,
                    Some(token.digest()),
                    event,
                )
                .await?
            }
            EventTarget::Operation(operation_id) => {
                self.deliver_by_operation(&mut transaction, *operation_id, None, event)
                    .await?
            }
            EventTarget::Job {
                provider,
                external_id,
            } => {
                sqlx::query_file_as!(
                    DeliveryRow,
                    "queries/operation_deliver_by_job.sql",
                    provider.as_str(),
                    self.server().as_str(),
                    external_id.as_str(),
                    event.event_id.as_str(),
                    event.payload,
                    event.is_final,
                    self.deployment().as_uuid(),
                )
                .fetch_one(&mut *transaction)
                .await?
            }
        };
        if let (Some(task), Some(key)) = (row.settled_task, row.settled_key.clone()) {
            super::wake(&mut transaction, self.server(), &[task], &[key]).await?;
        }
        transaction.commit().await?;
        Ok(classify(&row, event.is_final))
    }

    async fn deliver_by_operation(
        &self,
        transaction: &mut sqlx::PgConnection,
        operation_id: StepOperationId,
        digest: Option<Vec<u8>>,
        event: &OperationEvent,
    ) -> Result<DeliveryRow, TaskError> {
        Ok(sqlx::query_file_as!(
            DeliveryRow,
            "queries/operation_deliver_by_operation.sql",
            operation_id as StepOperationId,
            self.server().as_str(),
            digest,
            event.event_id.as_str(),
            event.payload,
            event.is_final,
            self.deployment().as_uuid(),
        )
        .fetch_one(transaction)
        .await?)
    }
}

fn classify(row: &DeliveryRow, is_final: bool) -> Delivery {
    if row.found == 0 {
        Delivery::Unknown
    } else if row.stored == 0 {
        Delivery::Repeat
    } else if is_final && (row.settled == 0 || row.finished == Some(true)) {
        Delivery::Late
    } else {
        Delivery::New
    }
}
