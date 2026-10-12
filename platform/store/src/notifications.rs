//! Change notifications: `kernel.notify_change` triggers send a JSON wake signal on the
//! schema's channel, and listeners read current rows under their own authorization.
use std::time::Duration;

use futures::{StreamExt, stream::BoxStream};
use serde::Deserialize;
use sqlx::postgres::{PgListener, PgNotification, PgPoolOptions};

use crate::{Channel, PlatformStore, StoreError, TenantId};

const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);

/// The row operation a notification reports.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, veoveo_types::Vocabulary)]
pub enum ChangeOperation {
    #[vocabulary(rename = "insert")]
    Insert,
    #[vocabulary(rename = "update")]
    Update,
    #[vocabulary(rename = "delete")]
    Delete,
}

/// One row change. The key and tenant locate the row; the consumer rereads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub channel: Channel,
    pub table: String,
    pub operation: ChangeOperation,
    /// Absent when the trigger names no key column or the key exceeds 1,024 bytes.
    pub key: Option<String>,
    pub tenant: Option<TenantId>,
}

/// What a listener receives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeSignal {
    /// Reread everything watched. Sent once the listener is subscribed and again after
    /// every reconnection, because PostgreSQL does not replay notifications sent while a
    /// listener was disconnected. A payload that does not decode also asks for it.
    Reconcile,
    Changed(Change),
}

/// Signals in arrival order. An error reports a failed connection; the stream retries
/// with backoff and sends [`ChangeSignal::Reconcile`] once subscribed again.
pub type ChangeStream = BoxStream<'static, Result<ChangeSignal, StoreError>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    table: String,
    operation: ChangeOperation,
    key: Option<String>,
    tenant: Option<uuid::Uuid>,
}

struct Listening {
    pool: sqlx::PgPool,
    channels: Vec<Channel>,
    listener: Option<PgListener>,
    failures: u32,
}

impl PlatformStore {
    /// Listen on `channels` through one connection of its own.
    pub fn listen(&self, channels: Vec<Channel>) -> ChangeStream {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .min_connections(0)
            .acquire_timeout(self.config().acquire_timeout())
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_lazy_with(self.connect_options().clone());
        let state = Listening {
            pool,
            channels,
            listener: None,
            failures: 0,
        };
        futures::stream::unfold(state, |mut state| async move {
            let signal = state.next().await;
            Some((signal, state))
        })
        .boxed()
    }
}

impl Listening {
    async fn next(&mut self) -> Result<ChangeSignal, StoreError> {
        let Some(listener) = self.listener.as_mut() else {
            if self.failures > 0 {
                tokio::time::sleep(reconnect_delay(self.failures)).await;
            }
            return match self.subscribe().await {
                Ok(listener) => {
                    self.listener = Some(listener);
                    self.failures = 0;
                    Ok(ChangeSignal::Reconcile)
                }
                Err(error) => {
                    self.failures = self.failures.saturating_add(1);
                    Err(error)
                }
            };
        };
        match listener.try_recv().await {
            Ok(Some(notification)) => Ok(decode(&notification)),
            // The listener reconnected and subscribed again before returning.
            Ok(None) => Ok(ChangeSignal::Reconcile),
            Err(error) => {
                self.listener = None;
                self.failures = self.failures.saturating_add(1);
                Err(error.into())
            }
        }
    }

    async fn subscribe(&self) -> Result<PgListener, StoreError> {
        let mut listener = PgListener::connect_with(&self.pool).await?;
        listener.ignore_pool_close_event(true);
        listener
            .listen_all(self.channels.iter().map(Channel::as_str))
            .await?;
        Ok(listener)
    }
}

fn reconnect_delay(failures: u32) -> Duration {
    let seconds = 1u64 << failures.saturating_sub(1).min(5);
    Duration::from_secs(seconds).min(MAX_RECONNECT_DELAY)
}

fn decode(notification: &PgNotification) -> ChangeSignal {
    let Ok(channel) = Channel::parse(notification.channel().to_owned()) else {
        return ChangeSignal::Reconcile;
    };
    match serde_json::from_str::<Payload>(notification.payload()) {
        Ok(payload) => ChangeSignal::Changed(Change {
            channel,
            table: payload.table,
            operation: payload.operation,
            key: payload.key,
            tenant: payload.tenant.map(TenantId::from_uuid),
        }),
        Err(_) => ChangeSignal::Reconcile,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_delay_doubles_to_thirty_seconds() {
        assert_eq!(reconnect_delay(1), Duration::from_secs(1));
        assert_eq!(reconnect_delay(2), Duration::from_secs(2));
        assert_eq!(reconnect_delay(5), Duration::from_secs(16));
        assert_eq!(reconnect_delay(6), Duration::from_secs(30));
        assert_eq!(reconnect_delay(60), Duration::from_secs(30));
    }

    #[test]
    fn payloads_admit_only_declared_members() {
        let payload: Payload = serde_json::from_str(
            r#"{"table":"note","operation":"delete","key":"1","tenant":null}"#,
        )
        .unwrap();
        assert_eq!(payload.operation, ChangeOperation::Delete);
        assert!(
            serde_json::from_str::<Payload>(
                r#"{"table":"note","operation":"truncate","key":null,"tenant":null}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<Payload>(
                r#"{"table":"note","operation":"insert","key":null,"tenant":null,"row":{}}"#
            )
            .is_err()
        );
    }
}
