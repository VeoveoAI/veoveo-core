//! Recovery after an instance dies, and fencing of the dead instance's writes.
use std::time::Duration;

use futures::StreamExt;
use veoveo_task::{RecoveryClass, TaskError, TaskFailure, TaskStatus, TaskTransition};

use crate::{
    store::TestDb,
    support::{claimed, kill_and_reap, runtime, within},
};

fn running() -> TaskTransition {
    TaskTransition::Running {
        message: "working".into(),
        progress: 0.4,
    }
}

#[tokio::test]
async fn a_reaped_instance_returns_resume_work_to_the_queue_and_loses_its_fence() {
    within(async {
        let db = TestDb::new().await;
        let doomed = runtime(&db).await;
        let survivor = runtime(&db).await;
        let task = claimed(&doomed, RecoveryClass::Resume).await;
        let progressed = doomed
            .transition_if_current(&task, running())
            .await
            .unwrap();
        let stale_writer = doomed.clone();

        kill_and_reap(&db, &doomed).await;

        let requeued = survivor.get(task.task_id).await.unwrap().unwrap();
        assert_eq!(requeued.status, TaskStatus::Queued);
        assert_eq!(requeued.owner_instance, None);
        assert_eq!(requeued.started_at, None);
        assert_eq!(requeued.progress, 0.0);
        assert!(requeued.revision > progressed.revision);

        // The dead instance's writes match no row.
        assert!(matches!(
            stale_writer
                .transition(
                    task.task_id,
                    TaskTransition::Succeeded {
                        message: "late".into(),
                        result: serde_json::json!({"late": true}),
                        result_uri: None,
                    }
                )
                .await,
            Err(TaskError::InvalidTransition {
                from: TaskStatus::Queued,
                ..
            })
        ));
        assert!(matches!(
            stale_writer
                .transition_if_current(&progressed, running())
                .await,
            Err(TaskError::NotOwner(_))
        ));

        // Another instance runs it again and settles it.
        let reclaimed = survivor.claim(task.task_id).await.unwrap();
        assert_eq!(reclaimed.owner_instance, Some(survivor.instance().id()));
        assert!(matches!(
            stale_writer
                .transition_if_current(&reclaimed, running())
                .await,
            Err(TaskError::NotOwner(_))
        ));
        let settled = survivor
            .transition_if_current(
                &reclaimed,
                TaskTransition::Succeeded {
                    message: "done".into(),
                    result: serde_json::json!({"ok": true}),
                    result_uri: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(settled.status, TaskStatus::Succeeded);
        assert_eq!(settled.owner_instance, None);
    })
    .await;
}

#[tokio::test]
async fn a_reaped_instance_fails_interrupted_indeterminate_work() {
    within(async {
        let db = TestDb::new().await;
        let doomed = runtime(&db).await;
        let survivor = runtime(&db).await;
        let task = claimed(&doomed, RecoveryClass::InterruptedIndeterminate).await;

        kill_and_reap(&db, &doomed).await;

        let failed = survivor.get(task.task_id).await.unwrap().unwrap();
        assert_eq!(failed.status, TaskStatus::Failed);
        assert_eq!(failed.error, Some(TaskFailure::interrupted_indeterminate()));
        assert!(failed.completed_at.is_some());
        assert_eq!(failed.owner_instance, None);
    })
    .await;
}

#[tokio::test]
async fn a_reaped_instance_completes_a_requested_cancellation() {
    within(async {
        let db = TestDb::new().await;
        let doomed = runtime(&db).await;
        let survivor = runtime(&db).await;
        let task = claimed(&doomed, RecoveryClass::Resume).await;
        let requested = survivor.cancel(task.task_id).await.unwrap();
        assert_eq!(requested.status, TaskStatus::CancelRequested);

        kill_and_reap(&db, &doomed).await;

        let cancelled = survivor.get(task.task_id).await.unwrap().unwrap();
        assert_eq!(cancelled.status, TaskStatus::Cancelled);
        assert!(cancelled.completed_at.is_some());
    })
    .await;
}

#[tokio::test]
async fn a_live_instance_keeps_its_work_through_a_reap() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let task = claimed(&worker, RecoveryClass::InterruptedIndeterminate).await;
        db.a.reap_instances().await.unwrap();
        let current = worker.get(task.task_id).await.unwrap().unwrap();
        assert_eq!(current, task);
    })
    .await;
}

#[tokio::test]
async fn the_work_stream_claims_work_a_dead_instance_released() {
    within(async {
        let db = TestDb::new().await;
        let doomed = runtime(&db).await;
        let survivor = runtime(&db).await;
        let task = claimed(&doomed, RecoveryClass::Resume).await;
        let mut work = survivor.work().await.unwrap();

        kill_and_reap(&db, &doomed).await;

        let reclaimed = tokio::time::timeout(Duration::from_secs(10), work.next())
            .await
            .expect("the work stream observed the released Task")
            .unwrap()
            .unwrap();
        assert_eq!(reclaimed.task_id, task.task_id);
        assert_eq!(reclaimed.owner_instance, Some(survivor.instance().id()));
    })
    .await;
}
