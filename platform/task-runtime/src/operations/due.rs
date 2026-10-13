//! The due pass: every process of a server queues the Tasks whose operations have a
//! check due or a deadline passed. Replicas share the work through `SKIP LOCKED`, and a
//! pass sleeps until the server's next due time.
use std::{sync::Arc, time::Duration};

use tokio::task::JoinHandle;
use veoveo_platform_store::{Instance, MaintenanceBatch, PlatformStore};
use veoveo_types::ServerSlug;

use crate::{TaskError, TaskRuntime};

/// The longest a process's due pass sleeps between passes, so it notices operations
/// that another process dispatched.
pub const DUE_INTERVAL: Duration = Duration::from_secs(5);

/// What one due pass did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DuePass {
    /// Parked Tasks queued because a check fell due.
    pub checks: u64,
    /// Operations whose deadline passed, now unresolved.
    pub unresolved: u64,
    /// Parked Tasks queued because their operation's deadline passed.
    pub expired: u64,
}

/// One runtime's due loop. Dropping the last clone of the runtime stops it, and so does
/// the loss of the runtime's instance.
pub(crate) struct Due(JoinHandle<()>);

impl Due {
    pub(crate) fn spawn(
        store: PlatformStore,
        server: ServerSlug,
        instance: &Arc<Instance>,
        interval: Duration,
    ) -> Self {
        let instance = Arc::downgrade(instance);
        Self(tokio::spawn(async move {
            loop {
                if instance.upgrade().is_none_or(|instance| instance.is_lost()) {
                    return;
                }
                let sleep = match pass(&store, &server, MaintenanceBatch::DEFAULT).await {
                    Ok((_, next)) => next.map_or(interval, |next| next.min(interval)),
                    Err(error) => {
                        tracing::warn!(server = server.as_str(), %error, "operation due pass failed");
                        interval
                    }
                };
                tokio::time::sleep(sleep.max(Duration::from_millis(10))).await;
            }
        }))
    }
}

impl Drop for Due {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Run due work in batches until none is left, and return how long until the next.
async fn pass(
    store: &PlatformStore,
    server: &ServerSlug,
    batch: MaintenanceBatch,
) -> Result<(DuePass, Option<Duration>), TaskError> {
    let limit = batch.limit();
    let mut done = DuePass::default();
    loop {
        let checks = sqlx::query_file!("queries/operation_due_checks.sql", server.as_str(), limit)
            .execute(store.pool())
            .await?
            .rows_affected();
        let deadlines = sqlx::query_file!(
            "queries/operation_due_deadlines.sql",
            server.as_str(),
            limit
        )
        .fetch_one(store.pool())
        .await?;
        done.checks += checks;
        done.unresolved += u64::try_from(deadlines.unresolved).unwrap_or_default();
        done.expired += u64::try_from(deadlines.woken).unwrap_or_default();
        if checks < u64::try_from(limit).unwrap_or(u64::MAX) && deadlines.unresolved < limit {
            break;
        }
    }
    let next = sqlx::query_file_scalar!("queries/operation_next_due.sql", server.as_str())
        .fetch_one(store.pool())
        .await?;
    let wait = next.map(|next| {
        (next - chrono::Utc::now())
            .to_std()
            .unwrap_or(Duration::ZERO)
    });
    Ok((done, wait))
}

impl TaskRuntime {
    /// Queue this server's Tasks whose operations have a check due or a deadline
    /// passed, in batches. The runtime runs it on its own whenever work falls due.
    pub async fn run_due(&self, batch: MaintenanceBatch) -> Result<DuePass, TaskError> {
        Ok(pass(self.platform_store(), self.server(), batch).await?.0)
    }
}
