//! Task retention. Every process of a server prunes the server's finished Tasks whose
//! retention expired and that no consumer pins, in batches that replicas share.
use std::{sync::Arc, time::Duration};

use tokio::{
    task::JoinHandle,
    time::{Instant, MissedTickBehavior},
};
use veoveo_store::{Instance, MaintenanceBatch, PlatformStore, Pruned};
use veoveo_types::ServerSlug;

use crate::{TaskError, TaskRuntime};

/// How often each process of a server prunes the server's expired Tasks.
pub const RETENTION_INTERVAL: Duration = Duration::from_secs(600);

/// One runtime's periodic retention pass. Dropping the last clone of the runtime stops
/// it, and so does the loss of the runtime's instance.
pub(crate) struct Retention(JoinHandle<()>);

impl Retention {
    pub(crate) fn spawn(
        store: PlatformStore,
        server: ServerSlug,
        instance: &Arc<Instance>,
        interval: Duration,
    ) -> Self {
        // The pass holds no strong reference, so it never keeps the instance alive.
        let instance = Arc::downgrade(instance);
        Self(tokio::spawn(async move {
            let mut ticks = tokio::time::interval_at(Instant::now() + interval, interval);
            ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticks.tick().await;
                if instance.upgrade().is_none_or(|instance| instance.is_lost()) {
                    return;
                }
                match prune_expired(&store, &server, MaintenanceBatch::DEFAULT).await {
                    Ok(pruned) => tracing::debug!(
                        server = server.as_str(),
                        rows = pruned.rows,
                        batches = pruned.batches,
                        "pruned expired Tasks"
                    ),
                    Err(error) => {
                        tracing::warn!(server = server.as_str(), %error, "Task retention failed");
                    }
                }
            }
        }))
    }
}

impl Drop for Retention {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn prune_expired(
    store: &PlatformStore,
    server: &ServerSlug,
    batch: MaintenanceBatch,
) -> Result<Pruned, TaskError> {
    Pruned::in_batches(batch, |limit| async move {
        Ok(
            sqlx::query_file!("queries/prune_expired.sql", server.as_str(), limit)
                .execute(store.pool())
                .await?
                .rows_affected(),
        )
    })
    .await
}

impl TaskRuntime {
    /// Delete this server's finished Tasks whose retention expired and that no consumer
    /// pins, in batches. Their input exchanges go with them. The runtime runs it every
    /// [`RETENTION_INTERVAL`] on its own.
    pub async fn prune_expired(&self, batch: MaintenanceBatch) -> Result<Pruned, TaskError> {
        prune_expired(self.platform_store(), self.server(), batch).await
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{
        TaskRuntime, TaskTransition,
        test_store::TestDb,
        test_support::{create, deployment, instance},
    };

    #[tokio::test]
    async fn the_runtime_prunes_expired_tasks_on_its_own() {
        tokio::time::timeout(Duration::from_secs(60), async {
            let db = TestDb::new().await;
            let server = veoveo_types::ServerSlug::parse("time").unwrap();
            let runtime = TaskRuntime::with_retention_interval(
                db.a.clone(),
                server.clone(),
                deployment(),
                instance(&db, "time").await,
                Duration::from_millis(100),
            );
            let expired = create(&runtime, Some(0)).await;
            let kept = create(&runtime, None).await;
            for task in [expired, kept] {
                let claimed = runtime.claim(task).await.unwrap();
                runtime
                    .transition_if_current(
                        &claimed,
                        TaskTransition::Succeeded {
                            message: "done".into(),
                            result: serde_json::json!({}),
                            result_uri: None,
                        },
                    )
                    .await
                    .unwrap();
            }
            while runtime.get(expired).await.unwrap().is_some() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            assert!(
                runtime.get(kept).await.unwrap().is_some(),
                "a Task within its retention stays"
            );
        })
        .await
        .expect("the case finishes within 60 seconds");
    }
}
