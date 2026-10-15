//! Work claims and the work stream.
use std::{collections::BTreeSet, time::Duration};

use crate::store::TestDb;
use futures::StreamExt;
use veoveo_task::{RecoveryClass, TaskError, TaskStatus};
use veoveo_types::TaskId;

use crate::support::{create, runtime, within};

#[tokio::test]
async fn concurrent_workers_claim_each_task_once() {
    within(async {
        let db = TestDb::new().await;
        let first = runtime(&db).await;
        let second = runtime(&db).await;
        let mut created = BTreeSet::new();
        for _ in 0..24 {
            created.insert(create(&first, RecoveryClass::Resume).await.task_id);
        }
        let drain = |runtime: veoveo_task::TaskRuntime| async move {
            let mut claimed = Vec::new();
            while let Some(task) = runtime.claim_next().await.unwrap() {
                assert_eq!(task.status, TaskStatus::Running);
                assert_eq!(task.owner_instance, Some(runtime.instance().id()));
                assert!(task.started_at.is_some());
                claimed.push(task.task_id);
            }
            claimed
        };
        let (from_first, from_second) = tokio::join!(drain(first.clone()), drain(second.clone()));
        let mut all = from_first
            .iter()
            .chain(&from_second)
            .copied()
            .collect::<Vec<_>>();
        all.sort();
        all.dedup();
        assert_eq!(
            all.len(),
            from_first.len() + from_second.len(),
            "a Task was claimed twice"
        );
        assert_eq!(all.into_iter().collect::<BTreeSet<_>>(), created);
        assert!(first.claim_next().await.unwrap().is_none());
    })
    .await;
}

#[tokio::test]
async fn claims_by_id_take_only_queued_tasks_of_this_server() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let rival = runtime(&db).await;
        let task = create(&worker, RecoveryClass::Resume).await;

        let claimed = worker.claim(task.task_id).await.unwrap();
        assert_eq!(claimed.status, TaskStatus::Running);
        assert_eq!(claimed.revision, task.revision + 1);
        assert!(matches!(
            rival.claim(task.task_id).await,
            Err(TaskError::InvalidTransition {
                from: TaskStatus::Running,
                to: TaskStatus::Running
            })
        ));
        assert!(matches!(
            worker.claim(TaskId::new()).await,
            Err(TaskError::NotFound(_))
        ));
        let foreign = crate::support::runtime_for(&db.a, "other").await;
        assert!(matches!(
            foreign.claim(task.task_id).await,
            Err(TaskError::NotFound(_))
        ));
        let version_four = TaskId::from_uuid(uuid::Uuid::new_v4());
        assert!(matches!(
            worker.claim(version_four).await,
            Err(TaskError::InvalidRecord(_))
        ));
    })
    .await;
}

#[tokio::test]
async fn the_work_stream_claims_queued_and_new_tasks() {
    within(async {
        let db = TestDb::new().await;
        let creator = runtime(&db).await;
        let worker = runtime(&db).await;
        let first = create(&creator, RecoveryClass::Resume).await;
        let second = create(&creator, RecoveryClass::InterruptedIndeterminate).await;

        let mut work = worker.work().await.unwrap();
        let claimed = [
            work.next().await.unwrap().unwrap(),
            work.next().await.unwrap().unwrap(),
        ];
        assert_eq!(
            claimed.iter().map(|task| task.task_id).collect::<Vec<_>>(),
            [first.task_id, second.task_id]
        );
        assert!(
            claimed
                .iter()
                .all(|task| task.owner_instance == Some(worker.instance().id()))
        );

        // A Task created later wakes the stream through the tasks channel.
        let third = create(&creator, RecoveryClass::Resume).await;
        let next = tokio::time::timeout(Duration::from_secs(10), work.next())
            .await
            .expect("the work stream observed the new Task")
            .unwrap()
            .unwrap();
        assert_eq!(next.task_id, third.task_id);

        // Another server's Task does not reach this stream.
        let other = crate::support::runtime_for(&db.a, "other").await;
        create(&other, RecoveryClass::Resume).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(500), work.next())
                .await
                .is_err()
        );
    })
    .await;
}
