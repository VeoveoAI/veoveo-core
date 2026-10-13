use crate::store::AuditStore;
use std::{sync::Arc, time::Duration};
use tokio::sync::{Mutex, mpsc, oneshot, watch};
use veoveo_audit_contract::AuditDraft;

const GROUP_LIMIT: usize = 64;
const QUEUE_LIMIT: usize = 1024;
const QUEUE_DEADLINE: Duration = Duration::from_secs(1);
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum AuditWriteError {
    #[error("audit writer is shutting down")]
    Closed,
    #[error("audit writer queue deadline exceeded")]
    QueueDeadline,
    #[error("required audit record could not be committed")]
    Commit,
}
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum AuditShutdownError {
    #[error("audit shutdown deadline exceeded; queued records were not fully committed")]
    Deadline,
    #[error("audit worker stopped unexpectedly; queued records may be uncommitted")]
    Worker,
}
struct Pending {
    draft: AuditDraft,
    reply: oneshot::Sender<Result<(), AuditWriteError>>,
}
#[derive(Debug)]
struct Drain {
    task: Option<tokio::task::JoinHandle<Result<(), AuditShutdownError>>>,
    result: Option<Result<(), AuditShutdownError>>,
}
#[derive(Debug)]
struct Runtime {
    stop: watch::Sender<bool>,
    drain: Mutex<Drain>,
}
#[derive(Debug, Clone)]
pub struct AuditWriter {
    sender: mpsc::Sender<Pending>,
    completions: mpsc::Sender<AuditDraft>,
    runtime: Arc<Runtime>,
}
impl AuditWriter {
    pub fn start(audit: AuditStore) -> Self {
        let (sender, receiver) = mpsc::channel::<Pending>(QUEUE_LIMIT);
        let (completions, completion_receiver) = mpsc::channel::<AuditDraft>(QUEUE_LIMIT);
        let (stop, stopped) = watch::channel(false);
        let task = tokio::spawn(async move {
            // One task owns both workers. A panic or forced shutdown drops both
            // receivers and rejects new writes; no detached child keeps running.
            tokio::try_join!(
                async {
                    required(audit.clone(), receiver, stopped.clone()).await;
                    Ok(())
                },
                async {
                    completed(audit.clone(), completion_receiver, stopped.clone()).await;
                    Ok::<(), AuditShutdownError>(())
                },
            )
            .map(|_| ())
        });
        Self {
            sender,
            completions,
            runtime: Arc::new(Runtime {
                stop,
                drain: Mutex::new(Drain {
                    task: Some(task),
                    result: None,
                }),
            }),
        }
    }
    pub fn is_running(&self) -> bool {
        !*self.runtime.stop.borrow() && !self.sender.is_closed() && !self.completions.is_closed()
    }
    /// Hosts stop admission if either worker loses its receiver.
    pub async fn closed(&self) {
        tokio::select! { _ = self.sender.closed() => {}, _ = self.completions.closed() => {} }
    }
    /// Stop producers first. Close both queues and wait for accepted records;
    /// replicas and other writer clones cannot keep the drain open indefinitely.
    pub async fn shutdown(&self, timeout: Duration) -> Result<(), AuditShutdownError> {
        self.runtime.stop.send_replace(true);
        let mut drain = self.runtime.drain.lock().await;
        if let Some(result) = drain.result {
            return result;
        }
        let task = drain
            .task
            .as_mut()
            .expect("audit drain owns its task until settlement");
        let result = match tokio::time::timeout(timeout, &mut *task).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(AuditShutdownError::Worker),
            Err(_) => {
                task.abort();
                let _ = task.await;
                Err(AuditShutdownError::Deadline)
            }
        };
        if let Err(error) = result {
            tracing::error!(%error, "audit shutdown incomplete");
        }
        drain.task = None;
        drain.result = Some(result);
        result
    }
    /// Completion follows an already executed effect. Queue pressure applies backpressure;
    /// failures retain the same identities for retry and never replace the tool result.
    pub async fn record_completion(&self, draft: AuditDraft) {
        if !self.is_running() || self.completions.send(draft).await.is_err() {
            tracing::error!("audit completion rejected after writer shutdown");
        }
    }
    /// Return only after this caller's record has committed. Cancelling the caller
    /// does not cancel a group containing other callers' records.
    pub async fn record(&self, draft: AuditDraft) -> Result<(), AuditWriteError> {
        if !self.is_running() {
            return Err(AuditWriteError::Closed);
        }
        let (reply, committed) = oneshot::channel();
        tokio::time::timeout(QUEUE_DEADLINE, self.sender.send(Pending { draft, reply }))
            .await
            .map_err(|_| AuditWriteError::QueueDeadline)?
            .map_err(|_| AuditWriteError::Closed)?;
        committed.await.map_err(|_| AuditWriteError::Closed)?
    }
}

async fn next<T>(receiver: &mut mpsc::Receiver<T>, stop: &mut watch::Receiver<bool>) -> Option<T> {
    loop {
        tokio::select! {
            biased;
            _ = stop.changed(), if !receiver.is_closed() => receiver.close(),
            record = receiver.recv() => return record,
        }
    }
}
async fn required(
    audit: AuditStore,
    mut receiver: mpsc::Receiver<Pending>,
    mut stop: watch::Receiver<bool>,
) {
    while let Some(first) = next(&mut receiver, &mut stop).await {
        let mut pending = vec![first];
        // Let other ready request tasks enqueue, then commit without imposing a
        // timer on an idle request. Writes arriving during this commit naturally
        // form the next group.
        tokio::task::yield_now().await;
        while pending.len() < GROUP_LIMIT {
            match receiver.try_recv() {
                Ok(next) => pending.push(next),
                Err(_) => break,
            }
        }
        let records: Vec<AuditDraft> = pending
            .iter()
            .map(|request| request.draft.clone())
            .collect();
        let started = tokio::time::Instant::now();
        let result = tokio::time::timeout(Duration::from_secs(15), audit.append(&records))
            .await
            .map_err(|_| AuditWriteError::Commit)
            .and_then(|result| result.map_err(|_| AuditWriteError::Commit));
        tracing::info!(
            audit_commit_ms = started.elapsed().as_secs_f64() * 1000.,
            audit_records = records.len(),
            audit_success = result.is_ok(),
            "audit group commit"
        );
        for request in pending {
            let _ = request.reply.send(result);
        }
    }
}
async fn completed(
    audit: AuditStore,
    mut receiver: mpsc::Receiver<AuditDraft>,
    mut stop: watch::Receiver<bool>,
) {
    while let Some(first) = next(&mut receiver, &mut stop).await {
        let mut records = vec![first];
        while records.len() < GROUP_LIMIT {
            match receiver.try_recv() {
                Ok(next) => records.push(next),
                Err(_) => break,
            }
        }
        let mut delay = Duration::from_millis(100);
        loop {
            match audit.append(&records).await {
                Ok(()) => break,
                Err(_) => {
                    tracing::error!(
                        audit_completion_pending = records.len(),
                        "audit completion commit unavailable; retaining queued records"
                    );
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(5));
                }
            }
        }
    }
}
