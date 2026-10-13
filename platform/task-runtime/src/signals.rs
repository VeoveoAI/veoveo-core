//! Wake signals from the runtime's server channel, `tasks.<server>`, which carries the
//! changes of that server's Tasks only. One listener connection per runtime fans signals
//! out to every subscription and work stream. A signal only names a changed Task;
//! readers reread current rows under their own authorization.
use std::collections::BTreeSet;

use futures::StreamExt;
use tokio::sync::{Mutex, broadcast};
use veoveo_platform_store::{
    Change, ChangeOperation, ChangeSignal, ChangeStream, Channel, PlatformStore,
};
use veoveo_types::TaskId;

use crate::TaskError;

/// The table whose changes `tasks.task_notify` reports.
const TASK_TABLE: &str = "task";
/// Signals a slow reader may fall behind by before it rereads everything it watches.
const BACKLOG: usize = 1024;

/// The runtime's shared listener. It starts with the first reader and stops after the
/// last one leaves.
#[derive(Default)]
pub(crate) struct SignalHub {
    source: Mutex<Option<broadcast::WeakSender<ChangeSignal>>>,
}

impl SignalHub {
    /// Join the shared listener on `channel`, starting it if no reader holds it. Returns
    /// once the listener is subscribed, so a read that follows misses no change.
    pub(crate) async fn subscribe(
        &self,
        store: &PlatformStore,
        channel: Channel,
    ) -> Result<TaskSignals, TaskError> {
        let mut source = self.source.lock().await;
        if let Some(sender) = source
            .as_ref()
            .and_then(broadcast::WeakSender::upgrade)
            .filter(|sender| sender.receiver_count() > 0)
        {
            return Ok(TaskSignals {
                receiver: sender.subscribe(),
            });
        }
        let mut changes = store.listen(vec![channel]);
        loop {
            match changes.next().await {
                Some(Ok(ChangeSignal::Reconcile)) => break,
                Some(Ok(ChangeSignal::Changed(_))) => {}
                Some(Err(error)) => return Err(error.into()),
                None => return Err(TaskError::ListenerStopped),
            }
        }
        let (sender, receiver) = broadcast::channel(BACKLOG);
        *source = Some(sender.downgrade());
        tokio::spawn(forward(changes, sender));
        Ok(TaskSignals { receiver })
    }
}

/// Forward signals until the last reader leaves. A failed connection is retried by the
/// listener, which then sends a reconcile.
async fn forward(mut changes: ChangeStream, sender: broadcast::Sender<ChangeSignal>) {
    loop {
        tokio::select! {
            () = sender.closed() => return,
            change = changes.next() => match change {
                Some(Ok(signal)) => {
                    let _ = sender.send(signal);
                }
                Some(Err(error)) => {
                    tracing::warn!(%error, "tasks notification listener reconnecting");
                }
                None => return,
            },
        }
    }
}

/// Every signal received since the previous wake.
#[derive(Default)]
pub(crate) struct TaskWake {
    /// Changes may have gone unnoticed: the listener reconnected or this reader fell
    /// behind.
    pub reconcile: bool,
    /// Tasks that changed.
    pub changed: BTreeSet<TaskId>,
    /// Some Task was created or changed, which may have made work claimable.
    pub claimable: bool,
}

impl TaskWake {
    fn add(&mut self, signal: ChangeSignal) {
        match signal {
            ChangeSignal::Reconcile => self.reconcile = true,
            ChangeSignal::Changed(Change {
                table,
                operation,
                key,
                ..
            }) if table == TASK_TABLE => {
                match key.as_deref().map(str::parse::<TaskId>) {
                    Some(Ok(id)) => {
                        self.changed.insert(id);
                    }
                    // Anyone may notify the channel; an unreadable key rereads everything.
                    _ => self.reconcile = true,
                }
                if operation != ChangeOperation::Delete {
                    self.claimable = true;
                }
            }
            ChangeSignal::Changed(_) => {}
        }
    }

    fn is_empty(&self) -> bool {
        !self.reconcile && self.changed.is_empty() && !self.claimable
    }
}

/// One reader of the runtime's shared listener.
pub(crate) struct TaskSignals {
    receiver: broadcast::Receiver<ChangeSignal>,
}

impl TaskSignals {
    /// Wait for a signal about Tasks, then merge every signal already received. `None`
    /// means the listener stopped.
    pub(crate) async fn next(&mut self) -> Option<TaskWake> {
        let mut wake = TaskWake::default();
        while wake.is_empty() {
            match self.receiver.recv().await {
                Ok(signal) => wake.add(signal),
                Err(broadcast::error::RecvError::Lagged(_)) => wake.reconcile = true,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
        loop {
            match self.receiver.try_recv() {
                Ok(signal) => wake.add(signal),
                Err(broadcast::error::TryRecvError::Lagged(_)) => wake.reconcile = true,
                Err(
                    broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed,
                ) => return Some(wake),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, time::Duration};

    use crate::{
        test_store::TestDb,
        test_support::{create, runtime},
    };

    #[tokio::test]
    async fn a_runtime_wakes_only_for_its_own_servers_tasks() {
        tokio::time::timeout(Duration::from_secs(60), async {
            let db = TestDb::new().await;
            let time = runtime(&db, "time").await;
            let other = runtime(&db, "other").await;
            let mut signals = time.signals().await.unwrap();
            for _ in 0..3 {
                create(&other, None).await;
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(500), signals.next())
                    .await
                    .is_err(),
                "another server's Tasks woke the runtime"
            );
            let own = create(&time, None).await;
            let wake = tokio::time::timeout(Duration::from_secs(10), signals.next())
                .await
                .expect("the runtime's own Task wakes it")
                .expect("the listener runs");
            assert_eq!(wake.changed, BTreeSet::from([own]));
            assert!(wake.claimable);
            assert!(!wake.reconcile);
        })
        .await
        .expect("the case finishes within 60 seconds");
    }
}
