//! Queued work for this runtime's server, claimed as the consumer asks for it.
use std::pin::Pin;

use futures::Stream;
use tokio::sync::watch;
use veoveo_platform_store::{ReapRound, StoreError};

use crate::{TaskError, TaskRuntime, TaskSnapshot, signals::TaskSignals};

/// Tasks this runtime's instance claimed, in the order it claimed them.
pub type TaskWork = Pin<Box<dyn Stream<Item = Result<TaskSnapshot, TaskError>> + Send + 'static>>;

impl TaskRuntime {
    /// Claim queued Tasks of this server, one per item the consumer polls.
    ///
    /// The stream subscribes to the `tasks` channel, claims every queued Task, then waits.
    /// A created or changed Task, each reap round of this instance, and a listener
    /// reconnect start another round of claims, so work that a dead instance returned to
    /// the queue is claimed again. Every item is owned by this runtime's instance; the
    /// consumer executes it and settles it with a worker transition. Replicas share the
    /// queue, and each Task goes to one of them. The stream ends with an error when the
    /// instance loses its lock, because its claims no longer hold.
    pub async fn work(&self) -> Result<TaskWork, TaskError> {
        let mut signals = self.signals().await?;
        let mut reaps = self.instance().reaps();
        let runtime = self.clone();
        Ok(Box::pin(async_stream::try_stream! {
            loop {
                runtime.check_instance()?;
                while let Some(task) = runtime.claim_next().await? {
                    yield task;
                }
                runtime.await_work(&mut signals, &mut reaps).await?;
            }
        }))
    }

    /// Wait until claimable work may exist.
    async fn await_work(
        &self,
        signals: &mut TaskSignals,
        reaps: &mut watch::Receiver<ReapRound>,
    ) -> Result<(), TaskError> {
        loop {
            tokio::select! {
                wake = signals.next() => match wake {
                    Some(wake) if wake.reconcile || wake.claimable => return Ok(()),
                    Some(_) => {}
                    None => return Err(TaskError::ListenerStopped),
                },
                reaped = reaps.changed() => {
                    reaped.map_err(|_| StoreError::InstanceLost)?;
                    return Ok(());
                }
                () = self.instance().lost() => return Err(StoreError::InstanceLost.into()),
            }
        }
    }
}
