//! Task changes notify only their server's channel.
use std::time::Duration;

use futures::StreamExt;
use veoveo_platform_store::{ChangeOperation, ChangeSignal, ChangeStream, Channel};
use veoveo_task_runtime::{RecoveryClass, TaskStatus};

use crate::{
    store::TestDb,
    support::{create, runtime_for, within},
};

async fn next_signal(changes: &mut ChangeStream) -> ChangeSignal {
    tokio::time::timeout(Duration::from_secs(10), changes.next())
        .await
        .expect("a signal within 10 seconds")
        .expect("the change stream continues")
        .expect("the listener is connected")
}

async fn quiet(changes: &mut ChangeStream) -> bool {
    tokio::time::timeout(Duration::from_millis(500), changes.next())
        .await
        .is_err()
}

#[tokio::test]
async fn task_changes_reach_only_their_servers_channel() {
    within(async {
        let db = TestDb::new().await;
        let time = runtime_for(&db.a, "time").await;
        let other = runtime_for(&db.a, "other").await;
        let channel = time.channel().unwrap();
        assert_eq!(channel.as_str(), "tasks.time");
        let mut own_changes = db.b.listen(vec![channel.clone()]);
        let mut schema_changes = db.b.listen(vec![Channel::TASKS]);
        assert_eq!(next_signal(&mut own_changes).await, ChangeSignal::Reconcile);
        assert_eq!(
            next_signal(&mut schema_changes).await,
            ChangeSignal::Reconcile
        );

        // PostgreSQL delivers notifications in commit order, so the other server's Task,
        // created first, would arrive first if it reached this channel.
        let foreign = create(&other, RecoveryClass::Resume).await;
        other.cancel(foreign.task_id).await.unwrap();
        let own = create(&time, RecoveryClass::Resume).await;
        let ChangeSignal::Changed(change) = next_signal(&mut own_changes).await else {
            panic!("expected the Task's insert");
        };
        assert_eq!(change.channel, channel);
        assert_eq!(change.table, "task");
        assert_eq!(change.operation, ChangeOperation::Insert);
        assert_eq!(change.key, Some(own.task_id.to_string()));
        assert!(change.tenant.is_some());

        let cancelled = time.cancel(own.task_id).await.unwrap();
        assert_eq!(cancelled.status, TaskStatus::Cancelled);
        let ChangeSignal::Changed(change) = next_signal(&mut own_changes).await else {
            panic!("expected the Task's update");
        };
        assert_eq!(
            (change.operation, change.key),
            (ChangeOperation::Update, Some(own.task_id.to_string()))
        );
        assert!(quiet(&mut own_changes).await);
        // No Task change reaches the schema-wide channel.
        assert!(quiet(&mut schema_changes).await);
    })
    .await;
}
