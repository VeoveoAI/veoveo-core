//! Deployment affinity is independent of logical schema, caller and process identity.
use std::time::Duration;

use serde_json::{Value, json};
use veoveo_task::{
    CheckObservation, CheckSchedule, Delivery, EventTarget, MaintenanceBatch, Operation,
    OperationCalls, OperationEvent, OperationEventId, OperationKey, OperationSpec, ProviderName,
    RecoveryClass, StepKey, Submission, TaskError, TaskPageOrder, TaskRuntime, TaskStatus,
    TaskTransition,
};
use veoveo_types::DeploymentId;

use crate::{
    store::TestDb,
    support::{claimed, draft, instance, kill_and_reap, owner, runtime, within},
};

#[tokio::test]
async fn distinct_deployments_share_no_claims_reads_or_idempotency_even_on_one_instance() {
    within(async {
        let db = TestDb::new().await;
        let first = runtime(&db).await;
        // Deliberately reuse the process instance: instance fencing alone cannot
        // enforce deployment affinity when the logical server and owner are identical.
        let other = TaskRuntime::new(
            db.a.clone(),
            first.server().clone(),
            DeploymentId::new(),
            first.instance().clone(),
        );
        let mut request = draft(&first, &owner(), RecoveryClass::Resume);
        request.idempotency_key = Some("same-request".into());
        let original = first.create(request.clone()).await.unwrap().snapshot;
        assert_eq!(original.deployment, first.deployment());
        let denied = sqlx::query_file!(
            "tests/queries/rewrite_deployment.sql",
            original.task_id.as_uuid(),
            other.deployment().as_uuid(),
        )
        .execute(db.a.pool())
        .await
        .unwrap_err();
        assert_eq!(
            denied
                .as_database_error()
                .and_then(|error| error.code())
                .as_deref(),
            Some("23514")
        );
        assert!(!first.create(request.clone()).await.unwrap().created);
        request.task_id = veoveo_types::TaskId::new();
        let independent = other.create(request).await.unwrap();
        assert!(independent.created);
        assert_ne!(independent.snapshot.task_id, original.task_id);
        assert_eq!(independent.snapshot.deployment, other.deployment());
        assert!(other.get(original.task_id).await.unwrap().is_none());
        assert!(
            other
                .get_many(&[original.task_id])
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            other
                .for_owner(&owner())
                .get(original.task_id)
                .await
                .unwrap()
                .is_none()
        );
        let page = other
            .for_owner(&owner())
            .page(TaskPageOrder::OldestFirst, None, 10)
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].task_id, independent.snapshot.task_id);
        assert!(matches!(
            other.claim(original.task_id).await,
            Err(TaskError::NotFound(_))
        ));
        assert_eq!(
            other.claim_next().await.unwrap().unwrap().task_id,
            independent.snapshot.task_id
        );
        assert!(other.claim_next().await.unwrap().is_none());
        let owned = first.claim(original.task_id).await.unwrap();
        assert!(matches!(
            other
                .transition_if_current(
                    &owned,
                    TaskTransition::Running {
                        message: "wrong deployment".into(),
                        progress: 0.5,
                    }
                )
                .await,
            Err(TaskError::NotOwner(_)) | Err(TaskError::NotFound(_))
        ));
        assert!(matches!(
            other.cancel(original.task_id).await,
            Err(TaskError::NotFound(_))
        ));
        let result: Result<Value, TaskError> = other
            .step(
                original.task_id,
                &StepKey::parse("forbidden").unwrap(),
                |_| async { panic!("another deployment must not execute a step") },
            )
            .await;
        assert!(matches!(result, Err(TaskError::NotFound(_))));
    })
    .await;
}

#[tokio::test]
async fn recovery_retains_deployment_and_indeterminate_work_is_not_reexecuted() {
    within(async {
        let db = TestDb::new().await;
        let first = runtime(&db).await;
        let replacement = TaskRuntime::new(
            db.a.clone(),
            first.server().clone(),
            first.deployment(),
            instance(&db.a, first.server().as_str()).await,
        );
        let other = TaskRuntime::new(
            db.a.clone(),
            first.server().clone(),
            DeploymentId::new(),
            instance(&db.a, first.server().as_str()).await,
        );
        let resumable = claimed(&first, RecoveryClass::Resume).await;
        let uncertain = claimed(&first, RecoveryClass::InterruptedIndeterminate).await;
        kill_and_reap(&db, &first).await;
        assert!(other.claim_next().await.unwrap().is_none());
        assert!(other.get(resumable.task_id).await.unwrap().is_none());
        let recovered = replacement.claim_next().await.unwrap().unwrap();
        assert_eq!(recovered.task_id, resumable.task_id);
        assert_eq!(recovered.deployment, first.deployment());
        assert_eq!(recovered.owner_instance, Some(replacement.instance().id()));
        assert_ne!(recovered.owner_instance, resumable.owner_instance);
        let uncertain = replacement.get(uncertain.task_id).await.unwrap().unwrap();
        assert_eq!(uncertain.status, TaskStatus::Failed);
        assert_eq!(uncertain.error.unwrap().code, "interrupted_indeterminate");
        assert!(replacement.claim_next().await.unwrap().is_none());
        assert!(matches!(
            first
                .transition_if_current(
                    &recovered,
                    TaskTransition::Running {
                        message: "stale instance".into(),
                        progress: 0.5,
                    }
                )
                .await,
            Err(TaskError::NotOwner(_))
        ));
    })
    .await;
}

struct NoProviderCall;
impl OperationCalls for NoProviderCall {
    type Error = TaskError;
    async fn check(&self, _: &Operation) -> Result<CheckObservation, TaskError> {
        panic!("the due pass must not execute provider work")
    }
    async fn cancel(&self, _: &Operation) -> Result<Value, TaskError> {
        panic!("the due pass must not execute provider work")
    }
}

#[tokio::test]
async fn shared_due_maintenance_does_not_transfer_execution_or_operation_access() {
    within(async {
        let db = TestDb::new().await;
        let first = runtime(&db).await;
        let other = TaskRuntime::new(
            db.a.clone(),
            first.server().clone(),
            DeploymentId::new(),
            instance(&db.a, first.server().as_str()).await,
        );
        let task = claimed(&first, RecoveryClass::Resume).await;
        let key = OperationKey::parse("check").unwrap();
        let spec = OperationSpec::checks(
            key.clone(),
            ProviderName::parse("fixture").unwrap(),
            CheckSchedule::new(Duration::from_secs(1), Duration::from_secs(1), 1.0, 2).unwrap(),
            Duration::from_secs(60),
        );
        let operation = first
            .dispatch_operation(task.task_id, &spec, |_| async {
                Ok::<_, TaskError>(Submission::handed_over())
            })
            .await
            .unwrap();
        assert!(matches!(
            first
                .wait_operation::<Value, _>(task.task_id, &key, &NoProviderCall)
                .await,
            Err(TaskError::Parked(_))
        ));
        assert!(other.operation(task.task_id, &key).await.unwrap().is_none());
        assert_eq!(
            other
                .deliver(&OperationEvent {
                    target: EventTarget::Operation(operation.operation_id),
                    event_id: OperationEventId::parse("wrong-deployment").unwrap(),
                    payload: json!({}),
                    is_final: true,
                })
                .await
                .unwrap(),
            Delivery::Unknown
        );
        loop {
            other.run_due(MaintenanceBatch::DEFAULT).await.unwrap();
            if first.get(task.task_id).await.unwrap().unwrap().status == TaskStatus::Queued {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(other.claim_next().await.unwrap().is_none());
        assert_eq!(
            first.claim_next().await.unwrap().unwrap().task_id,
            task.task_id
        );
    })
    .await;
}
