//! Creation, worker transitions, cancellation, resumable settlement and retention.
use std::collections::BTreeSet;

use crate::store::TestDb;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use veoveo_task::{
    MaintenanceBatch, Pruned, RecoveryClass, ResumeCancellationPolicy, TaskError, TaskFailure,
    TaskPayloadState, TaskRetentionPin, TaskStatus, TaskTransition,
};
use veoveo_types::{ResourceUri, ServerSlug, TenantId};

use crate::support::{OwnerSpec, claimed, create, draft, owner, owner_with, runtime, within};

fn success(result: serde_json::Value) -> TaskTransition {
    TaskTransition::Succeeded {
        message: "done".into(),
        result,
        result_uri: None,
    }
}

#[tokio::test]
async fn creation_validates_its_draft_and_queues_the_task() {
    within(async {
        let db = TestDb::new().await;
        let runtime = runtime(&db).await;
        let mut task = draft(&runtime, &owner(), RecoveryClass::Resume);
        task.ttl_ms = Some(60_000);
        task.retention_pins = BTreeSet::from([TaskRetentionPin::new("agent:episode").unwrap()]);
        let created = runtime.create(task.clone()).await.unwrap();
        assert!(created.created);
        let snapshot = created.snapshot;
        assert_eq!(snapshot.status, TaskStatus::Queued);
        assert_eq!(snapshot.status_message.as_deref(), Some("Queued"));
        assert_eq!(snapshot.owner, task.owner);
        assert_eq!(snapshot.request, task.request);
        assert_eq!(snapshot.ttl_ms, Some(60_000));
        assert_eq!(snapshot.poll_interval_ms, Some(1_000));
        assert_eq!(snapshot.retention_pins, task.retention_pins);
        assert_eq!(snapshot.revision, 1);
        assert_eq!(
            snapshot.retention_expires_at.unwrap() - snapshot.created_at,
            chrono::TimeDelta::milliseconds(60_000)
        );
        assert_eq!(
            runtime.owner(snapshot.task_id).await.unwrap(),
            Some(task.owner.clone())
        );
        assert_eq!(runtime.get(snapshot.task_id).await.unwrap(), Some(snapshot));

        let mut wrong_server = draft(&runtime, &owner(), RecoveryClass::Resume);
        wrong_server.server = ServerSlug::parse("other").unwrap();
        assert!(matches!(
            runtime.create(wrong_server).await,
            Err(TaskError::WrongServer(_))
        ));
        let mut foreign_context = owner();
        foreign_context.authority.tenant = TenantId::parse("another-tenant").unwrap();
        assert!(matches!(
            runtime
                .create(draft(&runtime, &foreign_context, RecoveryClass::Resume))
                .await,
            Err(TaskError::InvalidAuthority(_))
        ));
        let mut endless = draft(&runtime, &owner(), RecoveryClass::Resume);
        endless.ttl_ms = Some(u64::MAX);
        assert!(matches!(
            runtime.create(endless).await,
            Err(TaskError::InvalidRecord(_))
        ));
        let mut bad_key = draft(&runtime, &owner(), RecoveryClass::Resume);
        bad_key.idempotency_key = Some("line\nbreak".into());
        assert!(matches!(
            runtime.create(bad_key).await,
            Err(TaskError::InvalidRecord(_))
        ));
    })
    .await;
}

#[tokio::test]
async fn an_idempotency_key_returns_the_existing_task_for_the_same_owner() {
    within(async {
        let db = TestDb::new().await;
        let runtime = runtime(&db).await;
        let mut first = draft(&runtime, &owner(), RecoveryClass::Resume);
        first.idempotency_key = Some("request-1".into());
        let created = runtime.create(first.clone()).await.unwrap();
        assert!(created.created);

        let mut replay = draft(&runtime, &owner(), RecoveryClass::Resume);
        replay.idempotency_key = Some("request-1".into());
        let replayed = runtime.create(replay).await.unwrap();
        assert!(!replayed.created);
        assert_eq!(replayed.snapshot.task_id, created.snapshot.task_id);

        // The key is scoped to the owner's principal and surface.
        for spec in [
            OwnerSpec {
                principal: "copilot",
                ..OwnerSpec::default()
            },
            OwnerSpec {
                surface: "auditor",
                ..OwnerSpec::default()
            },
        ] {
            let mut other = draft(&runtime, &owner_with(spec), RecoveryClass::Resume);
            other.idempotency_key = Some("request-1".into());
            assert!(runtime.create(other).await.unwrap().created);
        }

        // Concurrent creations with one key produce one Task.
        let racers = (0..8).map(|_| {
            let runtime = runtime.clone();
            let mut task = draft(&runtime, &owner(), RecoveryClass::Resume);
            task.idempotency_key = Some("request-2".into());
            async move { runtime.create(task).await.unwrap() }
        });
        let results = futures::future::join_all(racers).await;
        assert_eq!(results.iter().filter(|result| result.created).count(), 1);
        assert_eq!(
            results
                .iter()
                .map(|result| result.snapshot.task_id)
                .collect::<BTreeSet<_>>()
                .len(),
            1
        );
    })
    .await;
}

#[tokio::test]
async fn results_and_failures_keep_absent_and_null_apart() {
    within(async {
        let db = TestDb::new().await;
        let runtime = runtime(&db).await;
        for result in [
            json!(null),
            json!(7),
            json!([1, null]),
            json!({"nested": {"u": u64::MAX}}),
        ] {
            let task = claimed(&runtime, RecoveryClass::Resume).await;
            let settled = runtime
                .transition_if_current(&task, success(result.clone()))
                .await
                .unwrap();
            assert_eq!(settled.result, Some(result.clone()));
            assert_eq!(settled.progress, 1.0);
            assert_eq!(
                runtime.payload_state(task.task_id).await.unwrap(),
                TaskPayloadState::Completed(result)
            );
        }

        let product = ResourceUri::new("time://schedules/example").unwrap();
        let task = claimed(&runtime, RecoveryClass::Resume).await;
        let settled = runtime
            .transition_if_current(
                &task,
                TaskTransition::Succeeded {
                    message: "done".into(),
                    result: json!({"schedule": true}),
                    result_uri: Some(product.clone()),
                },
            )
            .await
            .unwrap();
        assert_eq!(settled.result_uri, Some(product));

        for details in [
            None,
            Some(json!(null)),
            Some(json!({"provider": [1, null]})),
        ] {
            let task = claimed(&runtime, RecoveryClass::Resume).await;
            let failure = TaskFailure {
                details: details.clone(),
                ..TaskFailure::new("owner.failure", "it failed")
            };
            let failed = runtime
                .transition_if_current(&task, TaskTransition::Failed(failure.clone()))
                .await
                .unwrap();
            assert_eq!(failed.error, Some(failure.clone()));
            assert_eq!(failed.result, None);
            assert_eq!(
                runtime.get(task.task_id).await.unwrap().unwrap().error,
                Some(failure)
            );
        }
    })
    .await;
}

#[tokio::test]
async fn worker_transitions_require_the_current_revision_and_progress_bounds() {
    within(async {
        let db = TestDb::new().await;
        let runtime = runtime(&db).await;
        let task = claimed(&runtime, RecoveryClass::Resume).await;
        let progressed = runtime
            .transition_if_current(
                &task,
                TaskTransition::Running {
                    message: "half".into(),
                    progress: 0.5,
                },
            )
            .await
            .unwrap();
        assert_eq!(progressed.revision, task.revision + 1);
        assert!(matches!(
            runtime
                .transition_if_current(&task, success(json!({})))
                .await,
            Err(TaskError::Conflict(_))
        ));
        for progress in [f64::NAN, -0.1, 1.5] {
            assert!(matches!(
                runtime
                    .transition_if_current(
                        &progressed,
                        TaskTransition::Running {
                            message: "bad".into(),
                            progress,
                        }
                    )
                    .await,
                Err(TaskError::InvalidProgress)
            ));
        }
        let queued = create(&runtime, RecoveryClass::Resume).await;
        assert!(matches!(
            runtime
                .transition_if_current(&queued, success(json!({})))
                .await,
            Err(TaskError::InvalidTransition {
                from: TaskStatus::Queued,
                ..
            })
        ));
        let done = runtime
            .transition_if_current(&progressed, success(json!({})))
            .await
            .unwrap();
        assert!(matches!(
            runtime
                .transition_if_current(&done, success(json!({})))
                .await,
            Err(TaskError::InvalidTransition {
                from: TaskStatus::Succeeded,
                ..
            })
        ));
    })
    .await;
}

#[tokio::test]
async fn cancellation_finishes_unowned_tasks_and_signals_owned_ones() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let caller = runtime(&db).await;

        let queued = create(&worker, RecoveryClass::Resume).await;
        let cancelled = caller.cancel(queued.task_id).await.unwrap();
        assert_eq!(cancelled.status, TaskStatus::Cancelled);
        assert!(cancelled.cancel_requested_at.is_some() && cancelled.completed_at.is_some());
        assert!(worker.claim_next().await.unwrap().is_none());
        assert_eq!(caller.cancel(queued.task_id).await.unwrap(), cancelled);

        let task = claimed(&worker, RecoveryClass::Resume).await;
        let token = CancellationToken::new();
        let join = tokio::spawn(std::future::pending::<()>());
        worker
            .register_worker(task.task_id, token.clone(), join)
            .await
            .unwrap();
        let requested = worker.cancel(task.task_id).await.unwrap();
        assert_eq!(requested.status, TaskStatus::CancelRequested);
        assert_eq!(requested.owner_instance, Some(worker.instance().id()));
        assert!(token.is_cancelled());
        assert!(worker.is_cancel_requested(task.task_id).await.unwrap());
        assert!(matches!(
            caller
                .transition_if_current(&requested, TaskTransition::Cancelled)
                .await,
            Err(TaskError::NotOwner(_))
        ));
        let settled = worker
            .transition_if_current(&requested, TaskTransition::Cancelled)
            .await
            .unwrap();
        assert_eq!(settled.status, TaskStatus::Cancelled);
        assert_eq!(settled.owner_instance, None);
        assert_eq!(
            caller.payload_state(task.task_id).await.unwrap(),
            TaskPayloadState::Cancelled
        );
    })
    .await;
}

#[tokio::test]
async fn resumable_settlement_applies_cancellation_policy_and_stop_tokens() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let caller = runtime(&db).await;
        let failure = TaskFailure::new("owner.failure", "civil time is ambiguous");

        // Cancellation wins over a failure under CancellationWins.
        let task = claimed(&worker, RecoveryClass::Resume).await;
        caller.cancel(task.task_id).await.unwrap();
        let settled = worker
            .transition_resumable_if_current(
                &task,
                TaskTransition::Failed(failure.clone()),
                ResumeCancellationPolicy::CancellationWins,
                None,
            )
            .await
            .unwrap();
        assert_eq!(settled.status, TaskStatus::Cancelled);

        // PreserveFailure keeps a genuine failure but still suppresses success.
        let task = claimed(&worker, RecoveryClass::Resume).await;
        caller.cancel(task.task_id).await.unwrap();
        let settled = worker
            .transition_resumable(
                task.task_id,
                TaskTransition::Failed(failure.clone()),
                ResumeCancellationPolicy::PreserveFailure,
                None,
            )
            .await
            .unwrap();
        assert_eq!(settled.status, TaskStatus::Failed);
        assert_eq!(settled.error, Some(failure.clone()));
        let task = claimed(&worker, RecoveryClass::Resume).await;
        caller.cancel(task.task_id).await.unwrap();
        let settled = worker
            .transition_resumable_if_current(
                &task,
                success(json!({})),
                ResumeCancellationPolicy::PreserveFailure,
                None,
            )
            .await
            .unwrap();
        assert_eq!(settled.status, TaskStatus::Cancelled);

        // A settled Task is returned as it is.
        assert_eq!(
            worker
                .transition_resumable(
                    task.task_id,
                    success(json!({})),
                    ResumeCancellationPolicy::CancellationWins,
                    None,
                )
                .await
                .unwrap(),
            settled
        );

        // PreserveOutcome keeps a finished outcome, success or genuine failure.
        let task = claimed(&worker, RecoveryClass::Resume).await;
        caller.cancel(task.task_id).await.unwrap();
        let kept = worker
            .transition_resumable_if_current(
                &task,
                success(json!({"verified": true})),
                ResumeCancellationPolicy::PreserveOutcome,
                None,
            )
            .await
            .unwrap();
        assert_eq!(kept.status, TaskStatus::Succeeded);
        assert_eq!(kept.result, Some(json!({"verified": true})));
        let task = claimed(&worker, RecoveryClass::Resume).await;
        caller.cancel(task.task_id).await.unwrap();
        let kept = worker
            .transition_resumable(
                task.task_id,
                TaskTransition::Failed(failure.clone()),
                ResumeCancellationPolicy::PreserveOutcome,
                None,
            )
            .await
            .unwrap();
        assert_eq!(kept.status, TaskStatus::Failed);
        assert_eq!(kept.error, Some(failure.clone()));
        // Work that has not finished is cancelled under every policy.
        for policy in [
            ResumeCancellationPolicy::CancellationWins,
            ResumeCancellationPolicy::PreserveFailure,
            ResumeCancellationPolicy::PreserveOutcome,
        ] {
            let task = claimed(&worker, RecoveryClass::Resume).await;
            caller.cancel(task.task_id).await.unwrap();
            let unfinished = worker
                .transition_resumable(
                    task.task_id,
                    TaskTransition::Running {
                        message: "halfway".into(),
                        progress: 0.5,
                    },
                    policy,
                    None,
                )
                .await
                .unwrap();
            assert_eq!(unfinished.status, TaskStatus::Cancelled, "{policy}");
            let task = claimed(&worker, RecoveryClass::Resume).await;
            caller.cancel(task.task_id).await.unwrap();
            let cancelled = worker
                .transition_resumable(task.task_id, TaskTransition::Cancelled, policy, None)
                .await
                .unwrap();
            assert_eq!(cancelled.status, TaskStatus::Cancelled, "{policy}");
        }
        // A stopping worker still withholds success and settles a pending cancellation,
        // under PreserveOutcome as under the other policies.
        let stop = CancellationToken::new();
        stop.cancel();
        let task = claimed(&worker, RecoveryClass::Resume).await;
        caller.cancel(task.task_id).await.unwrap();
        let stopped = worker
            .transition_resumable(
                task.task_id,
                success(json!({})),
                ResumeCancellationPolicy::PreserveOutcome,
                Some(&stop),
            )
            .await
            .unwrap();
        assert_eq!(stopped.status, TaskStatus::Cancelled);
        let task = claimed(&worker, RecoveryClass::Resume).await;
        let unchanged = worker
            .transition_resumable(
                task.task_id,
                success(json!({})),
                ResumeCancellationPolicy::PreserveOutcome,
                Some(&stop),
            )
            .await
            .unwrap();
        assert_eq!(unchanged, task);

        // A stopped worker does not publish success.
        let task = claimed(&worker, RecoveryClass::Resume).await;
        let stop = CancellationToken::new();
        stop.cancel();
        let unchanged = worker
            .transition_resumable(
                task.task_id,
                success(json!({})),
                ResumeCancellationPolicy::CancellationWins,
                Some(&stop),
            )
            .await
            .unwrap();
        assert_eq!(unchanged, task);

        // Unrelated progress from a newer revision still conflicts.
        let progressed = worker
            .transition_if_current(
                &task,
                TaskTransition::Running {
                    message: "newer".into(),
                    progress: 0.5,
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            worker
                .transition_resumable_if_current(
                    &task,
                    success(json!({})),
                    ResumeCancellationPolicy::CancellationWins,
                    None,
                )
                .await,
            Err(TaskError::Conflict(_))
        ));

        // Another instance cannot settle it, and other recovery classes are refused.
        assert!(matches!(
            caller
                .transition_resumable_if_current(
                    &progressed,
                    success(json!({})),
                    ResumeCancellationPolicy::CancellationWins,
                    None,
                )
                .await,
            Err(TaskError::NotOwner(_))
        ));
        let indeterminate = claimed(&worker, RecoveryClass::InterruptedIndeterminate).await;
        assert!(matches!(
            worker
                .transition_resumable(
                    indeterminate.task_id,
                    success(json!({})),
                    ResumeCancellationPolicy::CancellationWins,
                    None,
                )
                .await,
            Err(TaskError::InvalidRecord(_))
        ));
    })
    .await;
}

#[tokio::test]
async fn pruning_removes_expired_unpinned_finished_tasks() {
    within(async {
        let db = TestDb::new().await;
        let runtime = runtime(&db).await;
        let pin = TaskRetentionPin::new("agent:episode").unwrap();
        let mut ids = Vec::new();
        for pins in [BTreeSet::new(), BTreeSet::from([pin.clone()])] {
            let mut task = draft(&runtime, &owner(), RecoveryClass::Resume);
            task.ttl_ms = Some(0);
            task.retention_pins = pins;
            task.idempotency_key = Some(format!("expiring-{}", ids.len()));
            let created = runtime.create(task).await.unwrap().snapshot;
            let claimed = runtime.claim(created.task_id).await.unwrap();
            runtime
                .transition_if_current(&claimed, success(json!({})))
                .await
                .unwrap();
            ids.push(created.task_id);
        }
        let mut unfinished = draft(&runtime, &owner(), RecoveryClass::Resume);
        unfinished.ttl_ms = Some(0);
        let unfinished = runtime.create(unfinished).await.unwrap().snapshot;

        let batch = MaintenanceBatch::new(1).unwrap();
        assert_eq!(
            runtime.prune_expired(batch).await.unwrap(),
            Pruned {
                rows: 1,
                batches: 2
            }
        );
        assert!(runtime.get(ids[0]).await.unwrap().is_none());
        assert!(
            runtime.get(ids[1]).await.unwrap().is_some(),
            "a pin keeps it"
        );
        assert!(runtime.get(unfinished.task_id).await.unwrap().is_some());

        // Pruning released the idempotency key.
        let mut reuse = draft(&runtime, &owner(), RecoveryClass::Resume);
        reuse.idempotency_key = Some("expiring-0".into());
        assert!(runtime.create(reuse).await.unwrap().created);

        let acknowledged = runtime
            .acknowledge_retention_pin(ids[1], &pin)
            .await
            .unwrap();
        assert!(acknowledged.retention_pins.is_empty());
        assert_eq!(
            runtime
                .acknowledge_retention_pin(ids[1], &pin)
                .await
                .unwrap(),
            acknowledged
        );
        assert_eq!(runtime.prune_expired(batch).await.unwrap().rows, 1);
        assert!(runtime.get(ids[1]).await.unwrap().is_none());
    })
    .await;
}

#[tokio::test]
async fn a_worker_leaves_the_registry_once_its_task_ends_however_it_ends() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let gone = |expected: usize| {
            let worker = worker.clone();
            async move {
                tokio::time::timeout(std::time::Duration::from_secs(10), async {
                    while worker.local_workers().await != expected {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                })
                .await
                .unwrap_or_else(|_| panic!("the registry holds {expected} workers"));
            }
        };

        // A worker that returns, one that panics and one that is aborted each leave the
        // registry without a sweep.
        let returned = claimed(&worker, RecoveryClass::Resume).await;
        worker
            .register_worker(
                returned.task_id,
                CancellationToken::new(),
                tokio::spawn(async {}),
            )
            .await
            .unwrap();
        gone(0).await;

        let panicked = claimed(&worker, RecoveryClass::Resume).await;
        worker
            .register_worker(
                panicked.task_id,
                CancellationToken::new(),
                tokio::spawn(async { panic!("the worker failed") }),
            )
            .await
            .unwrap();
        gone(0).await;

        let aborted = claimed(&worker, RecoveryClass::Resume).await;
        let join = tokio::spawn(std::future::pending::<()>());
        let abort = join.abort_handle();
        worker
            .register_worker(aborted.task_id, CancellationToken::new(), join)
            .await
            .unwrap();
        assert_eq!(worker.local_workers().await, 1);
        abort.abort();
        gone(0).await;

        // A Task that settles while its worker still runs keeps the entry until the worker
        // returns, so a cancellation can still reach the worker.
        let settled = claimed(&worker, RecoveryClass::Resume).await;
        let (finish, finished) = tokio::sync::oneshot::channel::<()>();
        worker
            .register_worker(
                settled.task_id,
                CancellationToken::new(),
                tokio::spawn(async move {
                    let _ = finished.await;
                }),
            )
            .await
            .unwrap();
        worker
            .transition(settled.task_id, success(json!({"ok": true})))
            .await
            .unwrap();
        assert_eq!(worker.local_workers().await, 1);
        finish.send(()).unwrap();
        gone(0).await;
    })
    .await;
}
