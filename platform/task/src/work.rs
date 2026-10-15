//! Queued work for this runtime's server, claimed as the consumer asks for it.
use std::{pin::Pin, time::Duration};

use futures::Stream;
use tokio::sync::watch;
use veoveo_store::{ReapRound, StoreError};

use crate::{TaskError, TaskRuntime, TaskSnapshot, signals::TaskSignals};

/// The wait after the first failed claim of a work stream. Each further failure in a row
/// doubles it, up to [`CLAIM_RETRY_MAX_DELAY`].
pub const CLAIM_RETRY_FIRST_DELAY: Duration = Duration::from_millis(100);
/// The longest wait between failed claims of a work stream.
pub const CLAIM_RETRY_MAX_DELAY: Duration = Duration::from_secs(5);

/// Tasks this runtime's instance claimed, in the order it claimed them.
pub type TaskWork = Pin<Box<dyn Stream<Item = Result<TaskSnapshot, TaskError>> + Send + 'static>>;

impl TaskRuntime {
    /// Claim queued Tasks of this server, one per item the consumer polls.
    ///
    /// The stream subscribes to its server's channel, `tasks.<server>`, claims every
    /// queued Task, then waits. A created or changed Task of this server, each reap round
    /// of this instance, and a listener reconnect start another round of claims, so work
    /// that a dead instance returned to the queue is claimed again. Another server's
    /// Tasks never wake it. Every item is owned by this runtime's instance; the
    /// consumer executes it and settles it with a worker transition. Replicas share the
    /// queue, and each Task goes to one of them. A claim that fails while the database
    /// connection is down is tried again with backoff, from [`CLAIM_RETRY_FIRST_DELAY`]
    /// to [`CLAIM_RETRY_MAX_DELAY`]. The stream ends with an error only when the
    /// instance ended, because its claims no longer hold.
    pub async fn work(&self) -> Result<TaskWork, TaskError> {
        let mut signals = self.signals().await?;
        let mut reaps = self.instance().reaps();
        let runtime = self.clone();
        Ok(Box::pin(async_stream::try_stream! {
            let mut failures = 0u32;
            loop {
                runtime.check_instance()?;
                match runtime.claim_next().await {
                    Ok(Some(task)) => {
                        failures = 0;
                        yield task;
                        continue;
                    }
                    Ok(None) => failures = 0,
                    Err(error) if runtime.instance().is_lost() => Err(error)?,
                    Err(error) => {
                        failures = failures.saturating_add(1);
                        tracing::warn!(
                            server = runtime.server().as_str(),
                            failures,
                            %error,
                            "claiming Tasks failed; trying again"
                        );
                        runtime.retry_after(failures).await?;
                        continue;
                    }
                }
                runtime.await_work(&mut signals, &mut reaps).await?;
            }
        }))
    }

    /// Wait before the next claim after `failures` failed claims in a row, unless the
    /// instance ends first.
    async fn retry_after(&self, failures: u32) -> Result<(), TaskError> {
        let delay = CLAIM_RETRY_FIRST_DELAY
            .saturating_mul(1 << failures.saturating_sub(1).min(16))
            .min(CLAIM_RETRY_MAX_DELAY);
        tokio::select! {
            () = tokio::time::sleep(delay) => Ok(()),
            () = self.instance().lost() => Err(StoreError::InstanceLost.into()),
        }
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
