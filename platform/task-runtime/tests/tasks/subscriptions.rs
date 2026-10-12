//! Caller subscriptions driven by notifications and listener reconnects.
use std::time::Duration;

use crate::store::TestDb;
use futures::StreamExt;
use veoveo_task_runtime::{
    RecoveryClass, TaskError, TaskSnapshot, TaskStatus, TaskTransition, TaskUpdateStream,
};
use veoveo_types::TaskId;

use crate::support::{OwnerSpec, claimed, interrupt_listeners, owner, owner_with, runtime, within};

async fn next(updates: &mut TaskUpdateStream) -> TaskSnapshot {
    tokio::time::timeout(Duration::from_secs(10), updates.next())
        .await
        .expect("a Task update arrived")
        .expect("the subscription is open")
        .expect("the update is readable")
}

async fn quiet(updates: &mut TaskUpdateStream) {
    assert!(
        tokio::time::timeout(Duration::from_millis(500), updates.next())
            .await
            .is_err(),
        "no update was expected"
    );
}

#[tokio::test]
async fn subscriptions_deliver_the_callers_tasks_as_they_change() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let listener = runtime(&db).await;
        let mine = claimed(&worker, RecoveryClass::Resume).await;
        let stranger = owner_with(OwnerSpec {
            principal: "stranger",
            ..OwnerSpec::default()
        });
        let theirs = worker
            .create(crate::support::draft(
                &worker,
                &stranger,
                RecoveryClass::Resume,
            ))
            .await
            .unwrap()
            .snapshot;

        let subscription = listener
            .for_owner(&owner())
            .subscribe(&[mine.task_id, theirs.task_id, TaskId::new()])
            .await
            .unwrap();
        assert_eq!(subscription.accepted_task_ids, [mine.task_id]);
        let mut updates = subscription.updates;
        assert_eq!(next(&mut updates).await, mine);

        // Changes to another caller's Task never reach this stream.
        worker.cancel(theirs.task_id).await.unwrap();
        quiet(&mut updates).await;

        // A worker on another instance changes the Task; the subscriber rereads it.
        let progressed = worker
            .transition_if_current(
                &mine,
                TaskTransition::Running {
                    message: "halfway".into(),
                    progress: 0.5,
                },
            )
            .await
            .unwrap();
        assert_eq!(next(&mut updates).await, progressed);
        let done = worker
            .transition_if_current(
                &progressed,
                TaskTransition::Succeeded {
                    message: "done".into(),
                    result: serde_json::json!({"ok": true}),
                    result_uri: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(next(&mut updates).await, done);
        assert_eq!(done.status, TaskStatus::Succeeded);
    })
    .await;
}

#[tokio::test]
async fn a_listener_reconnect_rereads_every_watched_task() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let listener = runtime(&db).await;
        let first = claimed(&worker, RecoveryClass::Resume).await;
        let second = claimed(&worker, RecoveryClass::Resume).await;
        let mut updates = listener
            .for_owner(&owner())
            .subscribe(&[first.task_id, second.task_id])
            .await
            .unwrap()
            .updates;
        next(&mut updates).await;
        next(&mut updates).await;

        // Drop the listener's connection; changes made meanwhile send no notification
        // to it, and the reconnect rereads both Tasks.
        interrupt_listeners(&db).await;
        let changed = worker
            .transition_if_current(
                &second,
                TaskTransition::Running {
                    message: "while disconnected".into(),
                    progress: 0.25,
                },
            )
            .await
            .unwrap();
        // Only the changed Task is delivered again; the unchanged revision is not.
        assert_eq!(next(&mut updates).await, changed);
        quiet(&mut updates).await;
    })
    .await;
}

#[tokio::test]
async fn subscriptions_admit_at_most_256_tasks() {
    within(async {
        let db = TestDb::new().await;
        let listener = runtime(&db).await;
        let ids = (0..257).map(|_| TaskId::new()).collect::<Vec<_>>();
        assert!(matches!(
            listener.for_owner(&owner()).subscribe(&ids).await,
            Err(TaskError::InvalidPageQuery)
        ));
        let empty = listener
            .for_owner(&owner())
            .subscribe(&ids[..3])
            .await
            .unwrap();
        assert!(empty.accepted_task_ids.is_empty());
    })
    .await;
}
