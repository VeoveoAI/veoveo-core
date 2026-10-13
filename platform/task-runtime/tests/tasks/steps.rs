//! Durable steps: recorded results, unsettled operations, owner fencing, retention and
//! row-level security.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::oneshot;
use veoveo_task_runtime::{
    MaintenanceBatch, RecoveryClass, StepAttempt, StepDispatch, StepKey, StepOperationId,
    TaskError, TaskInputRequest, TaskRuntime, TaskStatus, TaskTransition,
};
use veoveo_types::TaskId;

use crate::{
    store::TestDb,
    support::{claimed, create, draft, kill_and_reap, owner, runtime, runtime_for, slug, within},
};

#[derive(Debug)]
enum Failure {
    Task(TaskError),
    Provider,
}

impl From<TaskError> for Failure {
    fn from(error: TaskError) -> Self {
        Self::Task(error)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Charge {
    amount_cents: u64,
    receipt: String,
}

fn key(value: &str) -> StepKey {
    StepKey::parse(value).unwrap()
}

/// A step closure that fails the test if it runs.
async fn never<T>(_: StepAttempt) -> Result<T, Failure> {
    panic!("a finished step ran again")
}

async fn steps_of(store: &veoveo_platform_store::PlatformStore, task: TaskId) -> i64 {
    sqlx::query_file_scalar!("tests/queries/step_count.sql", task.as_uuid())
        .fetch_one(store.pool())
        .await
        .unwrap()
}

fn success() -> TaskTransition {
    TaskTransition::Succeeded {
        message: "done".into(),
        result: json!({}),
        result_uri: None,
    }
}

#[tokio::test]
async fn a_finished_step_returns_its_result_without_running_again() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let task = claimed(&worker, RecoveryClass::Resume).await;
        let runs = Arc::new(AtomicUsize::new(0));

        let charge = Charge {
            amount_cents: 1250,
            receipt: "r-1".into(),
        };
        let first = worker
            .step(task.task_id, &key("charge"), |attempt| {
                let runs = runs.clone();
                let charge = charge.clone();
                async move {
                    assert_eq!(attempt.dispatch, StepDispatch::First);
                    assert!(!attempt.is_unsettled());
                    runs.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, Failure>(charge)
                }
            })
            .await
            .unwrap();
        assert_eq!(first, charge);
        let again: Charge = worker
            .step(task.task_id, &key("charge"), never)
            .await
            .unwrap();
        assert_eq!(again, charge);
        assert_eq!(runs.load(Ordering::SeqCst), 1);

        // JSON null is a stored result: unit and an absent option both come back.
        worker
            .step(task.task_id, &key("notify"), |_| async {
                Ok::<_, Failure>(())
            })
            .await
            .unwrap();
        worker
            .step(task.task_id, &key("notify"), never::<()>)
            .await
            .unwrap();
        let none: Option<u32> = worker
            .step(task.task_id, &key("lookup"), |_| async {
                Ok::<_, Failure>(None)
            })
            .await
            .unwrap();
        assert_eq!(none, None);
        let stored: Option<u32> = worker
            .step(task.task_id, &key("lookup"), never)
            .await
            .unwrap();
        assert_eq!(stored, None);

        // A stored result decodes into the type the caller asks for.
        assert!(matches!(
            worker
                .step(task.task_id, &key("charge"), never::<u64>)
                .await,
            Err(Failure::Task(TaskError::Json(_)))
        ));
        assert_eq!(steps_of(&db.a, task.task_id).await, 3);
    })
    .await;
}

#[tokio::test]
async fn a_failed_attempt_leaves_its_operation_unsettled() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let task = claimed(&worker, RecoveryClass::Resume).await;
        let (sent, received) = oneshot::channel();

        let failed = worker
            .step(task.task_id, &key("charge"), |attempt| async move {
                sent.send(attempt.operation_id).unwrap();
                Err::<Charge, _>(Failure::Provider)
            })
            .await;
        assert!(matches!(failed, Err(Failure::Provider)));
        let operation = received.await.unwrap();

        let charge = worker
            .step(task.task_id, &key("charge"), |attempt| async move {
                assert_eq!(attempt.operation_id, operation);
                assert!(matches!(attempt.dispatch, StepDispatch::Unsettled { .. }));
                Ok::<_, Failure>(Charge {
                    amount_cents: 1250,
                    receipt: format!("reconciled-{operation}"),
                })
            })
            .await
            .unwrap();
        assert_eq!(charge.receipt, format!("reconciled-{operation}"));
        let again: Charge = worker
            .step(task.task_id, &key("charge"), never)
            .await
            .unwrap();
        assert_eq!(again, charge);
    })
    .await;
}

#[tokio::test]
async fn a_task_run_again_after_recovery_skips_its_finished_steps() {
    within(async {
        let db = TestDb::new().await;
        let doomed = runtime(&db).await;
        let survivor = runtime(&db).await;
        let task = claimed(&doomed, RecoveryClass::Resume).await;

        let reserved: u32 = doomed
            .step(task.task_id, &key("reserve"), |_| async {
                Ok::<_, Failure>(7)
            })
            .await
            .unwrap();
        // The process ends while the charge is in flight.
        let (sent, received) = oneshot::channel();
        let in_flight = tokio::spawn({
            let doomed = doomed.clone();
            async move {
                doomed
                    .step(task.task_id, &key("charge"), |attempt| async move {
                        sent.send(attempt.operation_id).unwrap();
                        std::future::pending::<Result<Charge, Failure>>().await
                    })
                    .await
            }
        });
        let operation = received.await.unwrap();
        in_flight.abort();
        let stale_writer = doomed.clone();
        kill_and_reap(&db, &doomed).await;

        let requeued = survivor.get(task.task_id).await.unwrap().unwrap();
        assert_eq!(requeued.status, TaskStatus::Queued);
        let reclaimed = survivor.claim(task.task_id).await.unwrap();
        let again: u32 = survivor
            .step(reclaimed.task_id, &key("reserve"), never)
            .await
            .unwrap();
        assert_eq!(again, reserved);
        let charge = survivor
            .step(reclaimed.task_id, &key("charge"), |attempt| async move {
                assert_eq!(attempt.operation_id, operation);
                assert!(attempt.is_unsettled());
                Ok::<_, Failure>(Charge {
                    amount_cents: 1250,
                    receipt: "r-1".into(),
                })
            })
            .await
            .unwrap();

        // The dead instance can neither read nor write the Task's steps.
        assert!(matches!(
            stale_writer
                .step(task.task_id, &key("charge"), never::<Charge>)
                .await,
            Err(Failure::Task(TaskError::NotOwner(_)))
        ));
        let finished: Charge = survivor
            .step(reclaimed.task_id, &key("charge"), never)
            .await
            .unwrap();
        assert_eq!(finished, charge);
    })
    .await;
}

#[tokio::test]
async fn step_writes_require_the_owning_instance() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let rival = runtime(&db).await;
        let task = claimed(&worker, RecoveryClass::Resume).await;

        assert!(matches!(
            rival.step(task.task_id, &key("charge"), never::<()>).await,
            Err(Failure::Task(TaskError::NotOwner(_)))
        ));
        let queued = create(&worker, RecoveryClass::Resume).await;
        assert!(matches!(
            worker
                .step(queued.task_id, &key("charge"), never::<()>)
                .await,
            Err(Failure::Task(TaskError::NotOwner(_)))
        ));
        assert!(matches!(
            worker
                .step(TaskId::new(), &key("charge"), never::<()>)
                .await,
            Err(Failure::Task(TaskError::NotFound(_)))
        ));
        assert_eq!(steps_of(&db.a, task.task_id).await, 0);

        // Waiting and cancel-requested Tasks still record their steps.
        worker
            .request_input(
                task.task_id,
                "confirm",
                TaskInputRequest {
                    kind: "elicitation/create".into(),
                    params: Default::default(),
                },
            )
            .await
            .unwrap();
        worker
            .step(task.task_id, &key("while-waiting"), |_| async {
                Ok::<_, Failure>(1)
            })
            .await
            .unwrap();
        let requested = rival.cancel(task.task_id).await.unwrap();
        assert_eq!(requested.status, TaskStatus::CancelRequested);
        worker
            .step(task.task_id, &key("while-cancelling"), |_| async {
                Ok::<_, Failure>(2)
            })
            .await
            .unwrap();

        // A finished Task has no owner.
        let finished = claimed(&worker, RecoveryClass::Resume).await;
        worker
            .transition_if_current(&finished, success())
            .await
            .unwrap();
        assert!(matches!(
            worker
                .step(finished.task_id, &key("late"), never::<()>)
                .await,
            Err(Failure::Task(TaskError::NotOwner(_)))
        ));
    })
    .await;
}

#[tokio::test]
async fn a_result_after_the_instance_lost_its_task_is_not_recorded() {
    within(async {
        let db = TestDb::new().await;
        let doomed = runtime(&db).await;
        let survivor = runtime(&db).await;
        let task = claimed(&doomed, RecoveryClass::Resume).await;
        let (sent, received) = oneshot::channel::<StepOperationId>();

        let late = doomed
            .step(task.task_id, &key("charge"), |attempt| {
                let db = &db;
                let doomed = &doomed;
                async move {
                    sent.send(attempt.operation_id).unwrap();
                    kill_and_reap(db, doomed).await;
                    Ok::<_, Failure>(Charge {
                        amount_cents: 1250,
                        receipt: "late".into(),
                    })
                }
            })
            .await;
        assert!(matches!(late, Err(Failure::Task(TaskError::NotOwner(_)))));
        let operation = received.await.unwrap();

        let reclaimed = survivor.claim(task.task_id).await.unwrap();
        survivor
            .step(reclaimed.task_id, &key("charge"), |attempt| async move {
                assert_eq!(attempt.operation_id, operation);
                assert!(attempt.is_unsettled());
                Ok::<_, Failure>(())
            })
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn steps_go_with_their_task() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let mut expiring = draft(&worker, &owner(), RecoveryClass::Resume);
        expiring.ttl_ms = Some(0);
        let created = worker.create(expiring).await.unwrap().snapshot;
        let task = worker.claim(created.task_id).await.unwrap();
        for name in ["first", "second"] {
            worker
                .step(task.task_id, &key(name), |_| async move {
                    Ok::<_, Failure>(name.to_owned())
                })
                .await
                .unwrap();
        }
        let current = worker.get(task.task_id).await.unwrap().unwrap();
        worker
            .transition_if_current(&current, success())
            .await
            .unwrap();
        assert_eq!(steps_of(&db.a, task.task_id).await, 2);

        assert_eq!(
            worker
                .prune_expired(MaintenanceBatch::DEFAULT)
                .await
                .unwrap()
                .rows,
            1
        );
        assert!(worker.get(task.task_id).await.unwrap().is_none());
        assert_eq!(steps_of(&db.a, task.task_id).await, 0);
    })
    .await;
}

#[tokio::test]
async fn the_step_table_keeps_each_operation_and_result() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let task = claimed(&worker, RecoveryClass::Resume).await;
        let insert = |key: String, operation: uuid::Uuid| {
            sqlx::query_file!(
                "tests/queries/insert_step.sql",
                task.task_id.as_uuid(),
                key,
                operation
            )
            .execute(db.a.pool())
        };
        assert!(insert("k".repeat(129), uuid::Uuid::now_v7()).await.is_err());
        assert!(insert(String::new(), uuid::Uuid::now_v7()).await.is_err());
        assert!(
            insert("line\nbreak".into(), uuid::Uuid::now_v7())
                .await
                .is_err()
        );
        assert!(insert("v4".into(), uuid::Uuid::new_v4()).await.is_err());
        insert("k".repeat(128), uuid::Uuid::now_v7()).await.unwrap();

        worker
            .step(task.task_id, &key("charge"), |_| async {
                Ok::<_, Failure>(5)
            })
            .await
            .unwrap();
        let rewrite = |operation: Option<uuid::Uuid>, result: Option<serde_json::Value>| {
            sqlx::query_file!(
                "tests/queries/rewrite_step.sql",
                task.task_id.as_uuid(),
                "charge",
                operation,
                result,
            )
            .execute(db.a.pool())
        };
        assert!(rewrite(Some(uuid::Uuid::now_v7()), None).await.is_err());
        assert!(rewrite(None, Some(json!(6))).await.is_err());
        let stored: i32 = worker
            .step(task.task_id, &key("charge"), never)
            .await
            .unwrap();
        assert_eq!(stored, 5);
    })
    .await;
}

#[tokio::test]
async fn row_level_security_keeps_another_server_out_of_steps() {
    within(async {
        let db = TestDb::new().await;
        let alpha = db.server_store(&slug("alpha")).await;
        let beta = db.server_store(&slug("beta")).await;
        let alpha_runtime = runtime_for(&alpha, "alpha").await;
        let beta_runtime = runtime_for(&beta, "beta").await;
        let alpha_task = claimed(&alpha_runtime, RecoveryClass::Resume).await;
        let beta_task = claimed(&beta_runtime, RecoveryClass::Resume).await;
        alpha_runtime
            .step(alpha_task.task_id, &key("charge"), |_| async {
                Ok::<_, Failure>(1)
            })
            .await
            .unwrap();

        // A runtime built for another server on alpha's login finds nothing.
        let intruder = TaskRuntime::new(
            alpha.clone(),
            slug("beta"),
            alpha_runtime.instance().clone(),
        );
        assert!(matches!(
            intruder
                .step(beta_task.task_id, &key("charge"), never::<()>)
                .await,
            Err(Failure::Task(TaskError::NotFound(_)))
        ));
        assert_eq!(steps_of(&alpha, alpha_task.task_id).await, 1);
        assert_eq!(steps_of(&beta, alpha_task.task_id).await, 0);
        assert_eq!(steps_of(&db.a, alpha_task.task_id).await, 1);
        assert!(
            sqlx::query_file!(
                "tests/queries/insert_step.sql",
                alpha_task.task_id.as_uuid(),
                "forged",
                uuid::Uuid::now_v7(),
            )
            .execute(beta.pool())
            .await
            .is_err()
        );
        assert_eq!(steps_of(&db.a, beta_task.task_id).await, 0);
    })
    .await;
}
