//! A server whose database sessions end keeps working: its instance takes its lock back
//! or continues as a new generation, its work stream and due pass reconnect, and only a
//! refused login ends its work.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use futures::StreamExt;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use veoveo_platform_store::{InstanceId, InstancePhase, LoginName, PlatformStore, StoreError};
use veoveo_task_runtime::{
    CheckSchedule, ExternalJobId, Operation, OperationCalls, OperationKey, OperationSpec,
    ProviderName, RecoveryClass, Submission, TaskError, TaskRuntime, TaskStatus,
};

use crate::{
    store::TestDb,
    support::{SERVER, claimed, create, runtime_for, slug, within},
};

/// A runtime of the default server on a login of its own, with a second pool of the
/// same login that ends the runtime's sessions.
async fn server_runtime(db: &TestDb) -> (TaskRuntime, PlatformStore) {
    let store = db.server_store(&slug(SERVER)).await;
    let ender = PlatformStore::connect(store.config().clone())
        .await
        .unwrap();
    (runtime_for(&store, SERVER).await, ender)
}

/// End every session of the login except the ender's, as a network failure would.
async fn end_sessions(ender: &PlatformStore) {
    let ended = sqlx::query_file_scalar!("tests/queries/end_login_sessions.sql")
        .fetch_one(ender.pool())
        .await
        .unwrap();
    assert!(ended > 0, "the runtime holds sessions");
}

async fn until_live(runtime: &TaskRuntime) {
    let mut changes = runtime.instance().changes();
    tokio::time::timeout(
        Duration::from_secs(30),
        changes.wait_for(|state| state.phase == InstancePhase::Live),
    )
    .await
    .expect("the instance is live again within 30 seconds")
    .unwrap();
}

#[tokio::test]
async fn a_runtime_keeps_claiming_after_its_sessions_end() {
    within(async {
        let db = TestDb::new().await;
        let (runtime, ender) = server_runtime(&db).await;
        let id = runtime.instance().id();
        let mut work = runtime.work().await.unwrap();
        let first = create(&runtime, RecoveryClass::Resume).await;
        let claimed = work.next().await.unwrap().unwrap();
        assert_eq!(claimed.task_id, first.task_id);

        end_sessions(&ender).await;
        let second = create(&runtime, RecoveryClass::Resume).await;
        let next = tokio::time::timeout(Duration::from_secs(30), work.next())
            .await
            .expect("the work stream claims after its sessions ended")
            .expect("the work stream continues")
            .unwrap();
        assert_eq!(next.task_id, second.task_id);

        // No reaper ran while the lock was free, so the instance kept its id and the
        // first Task is still its own.
        until_live(&runtime).await;
        assert_eq!(runtime.instance().id(), id);
        assert_eq!(runtime.instance().state().generation, 0);
        let kept = runtime.get(first.task_id).await.unwrap().unwrap();
        assert_eq!(kept.status, TaskStatus::Running);
        assert_eq!(kept.owner_instance, Some(id));
    })
    .await;
}

/// A provider whose checks never settle.
struct Pending;

impl OperationCalls for Pending {
    type Error = TaskError;

    async fn check(
        &self,
        _: &Operation,
    ) -> Result<veoveo_task_runtime::CheckObservation, TaskError> {
        Ok(veoveo_task_runtime::CheckObservation {
            event_id: None,
            payload: json!({"status": "processing"}),
            is_final: false,
        })
    }

    async fn cancel(&self, _: &Operation) -> Result<serde_json::Value, TaskError> {
        Ok(json!({}))
    }
}

#[tokio::test]
async fn the_due_pass_queues_a_check_after_its_sessions_end() {
    within(async {
        let db = TestDb::new().await;
        let (runtime, ender) = server_runtime(&db).await;
        let task = claimed(&runtime, RecoveryClass::Resume).await;
        let schedule =
            CheckSchedule::new(Duration::from_secs(2), Duration::from_secs(1), 1.0, 3).unwrap();
        let spec = OperationSpec::checks(
            OperationKey::parse("solve").unwrap(),
            ProviderName::parse("solver").unwrap(),
            schedule,
            Duration::from_secs(60),
        );
        runtime
            .dispatch_operation(task.task_id, &spec, |_| async {
                Ok::<_, TaskError>(Submission::job(ExternalJobId::parse("job-1").unwrap()))
            })
            .await
            .unwrap();
        let parked = runtime
            .wait_operation::<serde_json::Value, _>(task.task_id, &spec.key, &Pending)
            .await
            .unwrap_err();
        assert!(parked.is_parked());

        end_sessions(&ender).await;
        // The runtime's own due pass, on a new connection, queues the Task once its
        // check falls due.
        tokio::time::timeout(Duration::from_secs(30), async {
            while runtime.get(task.task_id).await.unwrap().unwrap().status != TaskStatus::Queued {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("the due pass queued the Task within 30 seconds");
    })
    .await;
}

/// Reports when the worker future that owns it is dropped, as an abort drops it.
struct Dropped(Arc<AtomicBool>);

impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn a_new_generation_stops_the_workers_of_the_old_one_before_it_claims() {
    within(async {
        let db = TestDb::new().await;
        let (runtime, _ender) = server_runtime(&db).await;
        let first = runtime.instance().id();
        create(&runtime, RecoveryClass::Resume).await;
        let task = runtime.claim_next().await.unwrap().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let guard = Dropped(stopped.clone());
        let join = tokio::spawn(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        runtime
            .register_worker(task.task_id, CancellationToken::new(), join)
            .await
            .unwrap();

        // A reaper deleted the row: the Task went to recovery, and the instance
        // continues as a new generation.
        sqlx::query_file!("tests/queries/delete_instance.sql", first as InstanceId)
            .execute(db.b.pool())
            .await
            .unwrap();
        let mut changes = runtime.instance().changes();
        tokio::time::timeout(
            Duration::from_secs(30),
            changes.wait_for(|state| state.generation == 1 && state.phase == InstancePhase::Live),
        )
        .await
        .expect("the instance continued as a new generation within 30 seconds")
        .unwrap();
        assert!(!stopped.load(Ordering::SeqCst));

        let reclaimed = runtime.claim_next().await.unwrap().unwrap();
        assert!(
            stopped.load(Ordering::SeqCst),
            "the earlier generation's worker stopped before the claim"
        );
        assert_eq!(reclaimed.task_id, task.task_id);
        assert_ne!(runtime.instance().id(), first);
        assert_eq!(reclaimed.owner_instance, Some(runtime.instance().id()));
    })
    .await;
}

#[tokio::test]
async fn a_runtime_whose_login_was_dropped_stops_claiming() {
    within(async {
        let db = TestDb::new().await;
        let (runtime, ender) = server_runtime(&db).await;
        let mut work = runtime.work().await.unwrap();
        let login = LoginName::parse(ender.config().username()).unwrap();
        // One session lists the others and later ends them.
        let mut session = ender.pool().acquire().await.unwrap();
        let sessions = sqlx::query_file_scalar!("tests/queries/login_sessions.sql")
            .fetch_all(&mut *session)
            .await
            .unwrap();
        // The Gateway removed the registration and dropped its login. A session of the
        // dropped role may still end the other sessions of that role.
        db.b.drop_server_login(&login).await.unwrap();
        for pid in sessions {
            sqlx::query_file_scalar!("tests/queries/terminate_backend.sql", pid)
                .fetch_one(&mut *session)
                .await
                .unwrap();
        }

        let ended = tokio::time::timeout(Duration::from_secs(30), work.next())
            .await
            .expect("the work stream ended within 30 seconds");
        assert!(matches!(
            ended,
            Some(Err(TaskError::Store(StoreError::InstanceLost)))
        ));
        assert!(runtime.instance().is_lost());
    })
    .await;
}

#[tokio::test]
async fn releasing_a_claim_hands_the_task_to_its_recovery() {
    within(async {
        let db = TestDb::new().await;
        let (runtime, _ender) = server_runtime(&db).await;

        let resumable = claimed(&runtime, RecoveryClass::Resume).await;
        let released = runtime.release(resumable.task_id).await.unwrap();
        assert_eq!(released.status, TaskStatus::Queued);
        assert_eq!(released.owner_instance, None);
        assert!(matches!(
            runtime.release(resumable.task_id).await,
            Err(TaskError::NotOwner(_))
        ));

        let indeterminate = claimed(&runtime, RecoveryClass::InterruptedIndeterminate).await;
        let interrupted = runtime.release(indeterminate.task_id).await.unwrap();
        assert_eq!(interrupted.status, TaskStatus::Failed);
        assert_eq!(
            interrupted.error.map(|failure| failure.code).as_deref(),
            Some("interrupted_indeterminate")
        );
    })
    .await;
}
