//! Operations: parked Tasks across restarts, delivery from any instance, repeated and
//! late events, concurrent final events, status checks, deadlines, cancellation,
//! unsettled dispatches, retention and row-level security.
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use veoveo_task::{
    CallbackToken, CheckObservation, CheckSchedule, Delivery, EventTarget, ExternalJobId,
    MaintenanceBatch, Operation, OperationCalls, OperationEvent, OperationEventId, OperationKey,
    OperationSpec, OperationState, ProviderName, RecoveryClass, StepAttempt, StepDispatch, StepKey,
    Submission, TaskError, TaskRuntime, TaskStatus, TaskTransition, WaitOutcome,
};
use veoveo_types::TaskId;

use crate::{
    store::TestDb,
    support::{claimed, draft, kill_and_reap, owner, runtime, runtime_for, slug, within},
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

impl Failure {
    fn parked(&self) -> bool {
        matches!(self, Self::Task(error) if error.is_parked())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Prediction {
    url: String,
}

/// A provider whose status checks answer from a script and whose cancellations count.
#[derive(Default)]
struct Provider {
    answers: Mutex<Vec<CheckObservation>>,
    checks: AtomicUsize,
    cancels: AtomicUsize,
}

impl OperationCalls for Provider {
    type Error = Failure;

    async fn check(&self, _: &Operation) -> Result<CheckObservation, Failure> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        let mut answers = self.answers.lock().unwrap();
        if answers.is_empty() {
            return Ok(CheckObservation {
                event_id: None,
                payload: json!({"status": "processing"}),
                is_final: false,
            });
        }
        Ok(answers.remove(0))
    }

    async fn cancel(&self, operation: &Operation) -> Result<Value, Failure> {
        self.cancels.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"cancelled": operation.external_id.as_ref().map(ExternalJobId::as_str)}))
    }
}

fn key(value: &str) -> OperationKey {
    OperationKey::parse(value).unwrap()
}

fn provider() -> ProviderName {
    ProviderName::parse("media").unwrap()
}

fn event_id(value: &str) -> OperationEventId {
    OperationEventId::parse(value).unwrap()
}

fn events(timeout: Duration) -> OperationSpec {
    OperationSpec::events(key("generate"), provider(), timeout)
}

/// Dispatch `spec` on `task`, keeping the callback token the first attempt received.
async fn dispatch(
    runtime: &TaskRuntime,
    task: TaskId,
    spec: &OperationSpec,
    job: &str,
) -> (Operation, Option<CallbackToken>) {
    let token = Arc::new(Mutex::new(None));
    let operation = runtime
        .dispatch_operation(task, spec, |attempt| {
            let token = token.clone();
            let job = job.to_owned();
            async move {
                *token.lock().unwrap() = attempt.callback;
                Ok::<_, Failure>(Submission::job(ExternalJobId::parse(job).unwrap()))
            }
        })
        .await
        .unwrap();
    let token = token.lock().unwrap().take();
    (operation, token)
}

async fn never(_: veoveo_task::OperationAttempt) -> Result<Submission, Failure> {
    panic!("a recorded operation was dispatched again")
}

async fn never_step(_: StepAttempt) -> Result<String, Failure> {
    panic!("a finished step ran again")
}

fn final_event(target: EventTarget, id: &str, url: &str) -> OperationEvent {
    OperationEvent {
        target,
        event_id: event_id(id),
        payload: json!({"url": url}),
        is_final: true,
    }
}

async fn events_of(db: &TestDb, task: TaskId) -> i64 {
    sqlx::query_file_scalar!("tests/queries/operation_event_count.sql", task.as_uuid())
        .fetch_one(db.a.pool())
        .await
        .unwrap()
}

/// Run the due pass until it queues `task`.
async fn until_queued(runtime: &TaskRuntime, task: TaskId) {
    loop {
        runtime.run_due(MaintenanceBatch::DEFAULT).await.unwrap();
        if runtime.get(task).await.unwrap().unwrap().status == TaskStatus::Queued {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_parked_task_survives_its_owner_and_resumes_where_it_waited() {
    within(async {
        let db = TestDb::new().await;
        let first = runtime(&db).await;
        let task = claimed(&first, RecoveryClass::Resume).await;
        let prepared: String = first
            .step(
                task.task_id,
                &StepKey::parse("prepare").unwrap(),
                |_| async { Ok::<_, Failure>("input.png".to_owned()) },
            )
            .await
            .unwrap();
        let spec = events(Duration::from_secs(3600));
        let (operation, token) = dispatch(&first, task.task_id, &spec, "job-1").await;
        assert_eq!(operation.state, OperationState::Submitted);
        assert_eq!(operation.external_id.as_ref().unwrap().as_str(), "job-1");
        let token = token.expect("the first attempt receives the callback token");
        let parked = first
            .wait_operation::<Prediction, _>(task.task_id, &spec.key, &Provider::default())
            .await
            .unwrap_err();
        assert!(parked.parked());
        let waiting = first.get(task.task_id).await.unwrap().unwrap();
        assert_eq!(waiting.status, TaskStatus::Waiting);
        assert_eq!(waiting.owner_instance, None);
        assert_eq!(waiting.status_message.as_deref(), Some("Waiting for media"));

        // The process ends; recovery leaves the parked Task waiting.
        kill_and_reap(&db, &first).await;
        let after = runtime(&db).await;
        let still = after.get(task.task_id).await.unwrap().unwrap();
        assert_eq!(still.status, TaskStatus::Waiting);
        assert_eq!(still.revision, waiting.revision);

        // A wrong token finds nothing; the right one settles and queues the Task.
        let wrong = final_event(
            EventTarget::Callback {
                operation_id: operation.operation_id,
                token: CallbackToken::parse(&"A".repeat(43)).unwrap(),
            },
            "evt-0",
            "https://wrong",
        );
        assert_eq!(after.deliver(&wrong).await.unwrap(), Delivery::Unknown);
        let callback = EventTarget::Callback {
            operation_id: operation.operation_id,
            token: token.clone(),
        };
        let done = final_event(callback.clone(), "evt-1", "https://out/1.png");
        assert_eq!(after.deliver(&done).await.unwrap(), Delivery::New);
        assert_eq!(after.deliver(&done).await.unwrap(), Delivery::Repeat);
        assert_eq!(
            after.get(task.task_id).await.unwrap().unwrap().status,
            TaskStatus::Queued
        );

        // Another instance resumes: finished steps and the dispatch replay from the
        // database, and the wait returns the outcome.
        let resumed = after.claim(task.task_id).await.unwrap();
        assert_eq!(resumed.status, TaskStatus::Running);
        let replayed: String = after
            .step(
                task.task_id,
                &StepKey::parse("prepare").unwrap(),
                never_step,
            )
            .await
            .unwrap();
        assert_eq!(replayed, prepared);
        let again = after
            .dispatch_operation(task.task_id, &spec, never)
            .await
            .unwrap();
        assert_eq!(again.operation_id, operation.operation_id);
        assert_eq!(again.state, OperationState::Settled);
        let outcome = after
            .wait_operation::<Prediction, _>(task.task_id, &spec.key, &Provider::default())
            .await
            .unwrap();
        assert_eq!(
            outcome,
            WaitOutcome::Settled(Prediction {
                url: "https://out/1.png".into()
            })
        );

        // A final event after settlement is stored as late and changes nothing.
        let late = final_event(callback, "evt-2", "https://out/2.png");
        assert_eq!(after.deliver(&late).await.unwrap(), Delivery::Late);
        assert_eq!(events_of(&db, task.task_id).await, 2);
        let current = after
            .operation(task.task_id, &spec.key)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.outcome, Some(json!({"url": "https://out/1.png"})));
    })
    .await;
}

#[tokio::test]
async fn the_first_final_event_wins_under_concurrency() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let task = claimed(&worker, RecoveryClass::Resume).await;
        let spec = events(Duration::from_secs(3600));
        dispatch(&worker, task.task_id, &spec, "job-race").await;
        let deliveries = (0..12).map(|n| {
            let worker = worker.clone();
            async move {
                let event = final_event(
                    EventTarget::Job {
                        provider: provider(),
                        external_id: ExternalJobId::parse("job-race").unwrap(),
                    },
                    &format!("race-{n}"),
                    &format!("https://out/{n}"),
                );
                (n, worker.deliver(&event).await.unwrap())
            }
        });
        let results = futures::future::join_all(deliveries).await;
        let winners: Vec<_> = results
            .iter()
            .filter(|(_, delivery)| *delivery == Delivery::New)
            .collect();
        assert_eq!(winners.len(), 1, "{results:?}");
        assert!(
            results
                .iter()
                .all(|(_, delivery)| matches!(delivery, Delivery::New | Delivery::Late))
        );
        let settled = worker
            .operation(task.task_id, &spec.key)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            settled.outcome,
            Some(json!({"url": format!("https://out/{}", winners[0].0)}))
        );
        assert_eq!(events_of(&db, task.task_id).await, 12);
    })
    .await;
}

/// Wait until `count` sessions of the test's database wait for a lock.
async fn until_lock_waits(db: &TestDb, count: i64) {
    loop {
        let waiting = sqlx::query_file_scalar!("tests/queries/lock_waits.sql")
            .fetch_one(db.a.pool())
            .await
            .unwrap();
        if waiting >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_final_event_that_reads_before_the_park_commits_queues_the_parked_task() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let receiver = runtime(&db).await;
        let task = claimed(&worker, RecoveryClass::Resume).await;
        let spec = events(Duration::from_secs(3600));
        let (operation, token) = dispatch(&worker, task.task_id, &spec, "job-park").await;
        let token = token.expect("the first attempt receives the callback token");

        // Another transaction holds the Task's row, so the park takes the operation's
        // row lock and then waits to change the Task.
        let mut holder = db.b.pool().begin().await.unwrap();
        sqlx::query_file_scalar!("tests/queries/lock_task.sql", task.task_id.as_uuid())
            .fetch_one(&mut *holder)
            .await
            .unwrap();
        let park = tokio::spawn({
            let worker = worker.clone();
            let key = spec.key.clone();
            async move {
                worker
                    .wait_operation::<Prediction, _>(task.task_id, &key, &Provider::default())
                    .await
            }
        });
        until_lock_waits(&db, 1).await;
        // The final event reads the database before the park commits, then waits for
        // the operation's row lock the park holds.
        let deliver = tokio::spawn({
            let event = final_event(
                EventTarget::Callback {
                    operation_id: operation.operation_id,
                    token,
                },
                "evt-park",
                "https://out/park.png",
            );
            async move { receiver.deliver(&event).await }
        });
        until_lock_waits(&db, 2).await;
        holder.commit().await.unwrap();

        assert!(park.await.unwrap().unwrap_err().parked());
        assert_eq!(deliver.await.unwrap().unwrap(), Delivery::New);
        let settled = worker
            .operation(task.task_id, &spec.key)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(settled.state, OperationState::Settled);
        assert_eq!(
            worker.get(task.task_id).await.unwrap().unwrap().status,
            TaskStatus::Queued,
            "the settled operation's Task stays parked"
        );
        worker.claim(task.task_id).await.unwrap();
        assert_eq!(
            worker
                .wait_operation::<Prediction, _>(task.task_id, &spec.key, &Provider::default())
                .await
                .unwrap(),
            WaitOutcome::Settled(Prediction {
                url: "https://out/park.png".into()
            })
        );
    })
    .await;
}

#[tokio::test]
async fn checks_follow_their_schedule_and_the_deadline_leaves_the_outcome_open() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        // Its retention ends as soon as it finishes.
        let mut expiring = draft(&worker, &owner(), RecoveryClass::Resume);
        expiring.ttl_ms = Some(0);
        let created = worker.create(expiring).await.unwrap().snapshot;
        let task = worker.claim(created.task_id).await.unwrap();
        let schedule =
            CheckSchedule::new(Duration::ZERO, Duration::from_millis(100), 2.0, 3).unwrap();
        let spec =
            OperationSpec::checks(key("solve"), provider(), schedule, Duration::from_secs(4));
        dispatch(&worker, task.task_id, &spec, "job-checked").await;
        let calls = Provider::default();
        let mut scheduled = Vec::new();
        for _ in 0..3 {
            let parked = worker
                .wait_operation::<Prediction, _>(task.task_id, &spec.key, &calls)
                .await
                .unwrap_err();
            assert!(parked.parked());
            let operation = worker
                .operation(task.task_id, &spec.key)
                .await
                .unwrap()
                .unwrap();
            scheduled.push(operation.clone());
            if operation.next_check_at.is_none() {
                break;
            }
            until_queued(&worker, task.task_id).await;
            worker.claim(task.task_id).await.unwrap();
        }
        assert_eq!(calls.checks.load(Ordering::SeqCst), 3);
        let last = scheduled.last().unwrap();
        assert_eq!(last.checks_made, 3);
        assert_eq!(last.next_check_at, None, "the check limit stops the checks");
        // The second interval doubles the first.
        let first_gap = scheduled[0].next_check_at.unwrap() - scheduled[0].dispatched_at;
        let second_gap = scheduled[1].next_check_at.unwrap() - scheduled[0].next_check_at.unwrap();
        assert!(second_gap > first_gap, "{first_gap} then {second_gap}");
        assert_eq!(events_of(&db, task.task_id).await, 3);

        // The deadline passes without an outcome: the operation is unresolved, the Task
        // resumes and learns the outcome is unknown.
        until_queued(&worker, task.task_id).await;
        let operation = worker
            .operation(task.task_id, &spec.key)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(operation.state, OperationState::Unresolved);
        worker.claim(task.task_id).await.unwrap();
        let expired = worker
            .wait_operation::<Prediction, _>(task.task_id, &spec.key, &calls)
            .await
            .unwrap();
        assert_eq!(expired, WaitOutcome::Expired);

        // The worker gives up, and retention keeps the Task while the outcome is open.
        let current = worker.get(task.task_id).await.unwrap().unwrap();
        worker
            .transition_if_current(
                &current,
                TaskTransition::Failed(veoveo_task::TaskFailure {
                    code: "provider_timeout".into(),
                    message: "The provider did not answer in time.".into(),
                    details: None,
                }),
            )
            .await
            .unwrap();
        worker
            .prune_expired(MaintenanceBatch::DEFAULT)
            .await
            .unwrap();
        assert!(worker.get(task.task_id).await.unwrap().is_some());

        // A late final event still settles it, and retention then removes the Task.
        let late = final_event(
            EventTarget::Job {
                provider: provider(),
                external_id: ExternalJobId::parse("job-checked").unwrap(),
            },
            "late-1",
            "https://out/late",
        );
        assert_eq!(worker.deliver(&late).await.unwrap(), Delivery::Late);
        let settled = worker
            .operation(task.task_id, &spec.key)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(settled.state, OperationState::Settled);
        worker
            .prune_expired(MaintenanceBatch::DEFAULT)
            .await
            .unwrap();
        assert!(worker.get(task.task_id).await.unwrap().is_none());
    })
    .await;
}

#[tokio::test]
async fn a_final_check_settles_the_operation() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let task = claimed(&worker, RecoveryClass::Resume).await;
        let schedule = CheckSchedule::new(Duration::ZERO, Duration::from_secs(60), 1.0, 5).unwrap();
        let spec =
            OperationSpec::checks(key("solve"), provider(), schedule, Duration::from_secs(600));
        dispatch(&worker, task.task_id, &spec, "job-done").await;
        let calls = Provider::default();
        calls.answers.lock().unwrap().push(CheckObservation {
            event_id: Some(event_id("status-7")),
            payload: json!({"url": "https://out/solved"}),
            is_final: true,
        });
        let outcome = worker
            .wait_operation::<Prediction, _>(task.task_id, &spec.key, &calls)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            WaitOutcome::Settled(Prediction {
                url: "https://out/solved".into()
            })
        );
        assert_eq!(calls.checks.load(Ordering::SeqCst), 1);
    })
    .await;
}

#[tokio::test]
async fn cancelling_a_parked_task_carries_the_cancellation_to_the_provider() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let task = claimed(&worker, RecoveryClass::Resume).await;
        let spec = events(Duration::from_secs(3600));
        let (operation, token) = dispatch(&worker, task.task_id, &spec, "job-cancel").await;
        let calls = Provider::default();
        assert!(
            worker
                .wait_operation::<Prediction, _>(task.task_id, &spec.key, &calls)
                .await
                .unwrap_err()
                .parked()
        );
        let requested = worker.cancel(task.task_id).await.unwrap();
        assert_eq!(requested.status, TaskStatus::CancelRequested);
        assert_eq!(requested.owner_instance, None);

        // A claim keeps the cancellation and gives the Task to a worker.
        let claimed = worker.claim(task.task_id).await.unwrap();
        assert_eq!(claimed.status, TaskStatus::CancelRequested);
        let outcome = worker
            .wait_operation::<Prediction, _>(task.task_id, &spec.key, &calls)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            WaitOutcome::Cancelled {
                receipt: json!({"cancelled": "job-cancel"})
            }
        );
        assert_eq!(calls.cancels.load(Ordering::SeqCst), 1);
        let stored = worker
            .operation(task.task_id, &spec.key)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.cancel_receipt,
            Some(json!({"cancelled": "job-cancel"}))
        );
        let current = worker.get(task.task_id).await.unwrap().unwrap();
        let cancelled = worker
            .transition_if_current(&current, TaskTransition::Cancelled)
            .await
            .unwrap();
        assert_eq!(cancelled.status, TaskStatus::Cancelled);

        // The provider's final event still arrives and is stored as late.
        let late = final_event(
            EventTarget::Callback {
                operation_id: operation.operation_id,
                token: token.unwrap(),
            },
            "after-cancel",
            "https://out/unused",
        );
        assert_eq!(worker.deliver(&late).await.unwrap(), Delivery::Late);
        assert_eq!(events_of(&db, task.task_id).await, 1);
    })
    .await;
}

#[tokio::test]
async fn an_owner_cancelling_a_parked_task_leaves_the_cancellation_to_a_claim() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let task = claimed(&worker, RecoveryClass::Resume).await;
        let spec = events(Duration::from_secs(3600));
        dispatch(&worker, task.task_id, &spec, "job-owner-cancel").await;
        let calls = Provider::default();
        assert!(
            worker
                .wait_operation::<Prediction, _>(task.task_id, &spec.key, &calls)
                .await
                .unwrap_err()
                .parked()
        );

        // The parked Task has no owner, yet its operation still runs at the provider:
        // the caller's cancellation is recorded for the next claim.
        let requested = worker
            .for_owner(&owner())
            .cancel(task.task_id)
            .await
            .unwrap();
        assert_eq!(requested.status, TaskStatus::CancelRequested);
        assert_eq!(requested.owner_instance, None);
        assert_eq!(requested.completed_at, None);
        assert_eq!(calls.cancels.load(Ordering::SeqCst), 0);

        let claimed = worker.claim(task.task_id).await.unwrap();
        assert_eq!(claimed.status, TaskStatus::CancelRequested);
        let outcome = worker
            .wait_operation::<Prediction, _>(task.task_id, &spec.key, &calls)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            WaitOutcome::Cancelled {
                receipt: json!({"cancelled": "job-owner-cancel"})
            }
        );
        assert_eq!(calls.cancels.load(Ordering::SeqCst), 1);
    })
    .await;
}

#[tokio::test]
async fn a_dispatch_without_a_recorded_submission_is_reported_unsettled() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let task = claimed(&worker, RecoveryClass::Resume).await;
        let spec = events(Duration::from_secs(3600));
        let first_id = Arc::new(Mutex::new(None));
        let failed = worker
            .dispatch_operation(task.task_id, &spec, |attempt| {
                let first_id = first_id.clone();
                async move {
                    assert_eq!(attempt.dispatch, StepDispatch::First);
                    assert!(attempt.callback.is_some());
                    *first_id.lock().unwrap() = Some(attempt.operation_id);
                    Err::<Submission, _>(Failure::Provider)
                }
            })
            .await;
        assert!(matches!(failed, Err(Failure::Provider)));
        let pending = worker
            .operation(task.task_id, &spec.key)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pending.state, OperationState::Dispatching);

        let reconciled = worker
            .dispatch_operation(task.task_id, &spec, |attempt| async move {
                assert!(matches!(attempt.dispatch, StepDispatch::Unsettled { .. }));
                assert!(
                    attempt.callback.is_none(),
                    "the provider holds the first token"
                );
                Ok::<_, Failure>(Submission::handed_over())
            })
            .await
            .unwrap();
        assert_eq!(Some(reconciled.operation_id), *first_id.lock().unwrap());
        assert_eq!(reconciled.state, OperationState::Submitted);
        assert_eq!(reconciled.external_id, None);

        // An unknown outcome can still settle through the operation's identity.
        let event = final_event(
            EventTarget::Operation(reconciled.operation_id),
            "by-identity",
            "https://out/x",
        );
        assert_eq!(worker.deliver(&event).await.unwrap(), Delivery::New);
    })
    .await;
}

#[tokio::test]
async fn another_server_sees_and_settles_none_of_a_servers_operations() {
    within(async {
        let db = TestDb::new().await;
        let alpha_store = db.server_store(&slug("alpha")).await;
        let beta_store = db.server_store(&slug("beta")).await;
        let alpha = runtime_for(&alpha_store, "alpha").await;
        let beta = runtime_for(&beta_store, "beta").await;
        let task = claimed(&alpha, RecoveryClass::Resume).await;
        let spec = events(Duration::from_secs(3600));
        let (operation, _) = dispatch(&alpha, task.task_id, &spec, "job-alpha").await;
        assert!(
            beta.operation(task.task_id, &spec.key)
                .await
                .unwrap()
                .is_none()
        );
        let event = final_event(
            EventTarget::Operation(operation.operation_id),
            "foreign",
            "https://out/foreign",
        );
        assert_eq!(beta.deliver(&event).await.unwrap(), Delivery::Unknown);
        assert_eq!(alpha.deliver(&event).await.unwrap(), Delivery::New);
    })
    .await;
}
