//! Input requests and caller answers.
use std::collections::BTreeMap;

use crate::store::TestDb;
use serde_json::{Value, json};
use veoveo_task_runtime::{
    MAX_TASK_INPUTS, RecoveryClass, TaskError, TaskInputRequest, TaskInputSubmission, TaskStatus,
};

use crate::support::{OwnerSpec, claimed, create, owner, owner_with, runtime, within};

fn request(message: &str) -> TaskInputRequest {
    TaskInputRequest {
        kind: "elicitation/create".into(),
        params: BTreeMap::from([("message".to_owned(), json!(message))]),
    }
}

fn answers(entries: &[(&str, Value)]) -> BTreeMap<String, BTreeMap<String, Value>> {
    entries
        .iter()
        .map(|(key, value)| {
            (
                (*key).to_owned(),
                BTreeMap::from([("action".to_owned(), value.clone())]),
            )
        })
        .collect()
}

#[tokio::test]
async fn a_worker_asks_and_its_caller_answers_once() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let caller = runtime(&db).await;
        let task = claimed(&worker, RecoveryClass::Resume).await;

        let exchange = worker
            .request_input(task.task_id, "confirm", request("Proceed?"))
            .await
            .unwrap();
        assert_eq!(exchange.key, "confirm");
        assert_eq!(exchange.response, None);
        let waiting = worker.get(task.task_id).await.unwrap().unwrap();
        assert_eq!(waiting.status, TaskStatus::Waiting);
        assert!(matches!(
            worker
                .request_input(task.task_id, "confirm", request("Again?"))
                .await,
            Err(TaskError::DuplicateInputKey(_))
        ));
        assert!(matches!(
            caller
                .request_input(task.task_id, "other", request("Not mine"))
                .await,
            Err(TaskError::NotOwner(_))
        ));
        assert!(matches!(
            worker
                .request_input(task.task_id, "", request("Empty key"))
                .await,
            Err(TaskError::InvalidInputKey)
        ));
        worker
            .request_input(task.task_id, "second", request("Second?"))
            .await
            .unwrap();
        assert_eq!(
            worker
                .outstanding_inputs(task.task_id)
                .await
                .unwrap()
                .into_keys()
                .collect::<Vec<_>>(),
            ["confirm", "second"]
        );

        // Another principal cannot see or answer the Task.
        let stranger = caller.for_owner(&owner_with(OwnerSpec {
            principal: "stranger",
            ..OwnerSpec::default()
        }));
        assert!(matches!(
            stranger
                .submit_input_responses(task.task_id, answers(&[("confirm", json!("accept"))]))
                .await,
            Err(TaskError::NotFound(_))
        ));

        let query = caller.for_owner(&owner());
        let submission = query
            .submit_input_responses(
                task.task_id,
                answers(&[("confirm", json!("accept")), ("unknown", json!("accept"))]),
            )
            .await
            .unwrap();
        assert_eq!(
            submission,
            TaskInputSubmission {
                accepted: 1,
                ignored: 1
            }
        );
        let answered = worker.get(task.task_id).await.unwrap().unwrap();
        assert_eq!(answered.revision, waiting.revision + 2);
        let exchange = worker
            .input_exchange(task.task_id, "confirm")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            exchange.response,
            Some(BTreeMap::from([("action".to_owned(), json!("accept"))]))
        );
        assert!(exchange.responded_at.is_some());

        // A repeated answer is ignored and leaves the Task unchanged.
        let repeat = query
            .submit_input_responses(task.task_id, answers(&[("confirm", json!("decline"))]))
            .await
            .unwrap();
        assert_eq!(repeat.ignored, 1);
        assert_eq!(worker.get(task.task_id).await.unwrap().unwrap(), answered);
        assert_eq!(
            worker
                .outstanding_inputs(task.task_id)
                .await
                .unwrap()
                .into_keys()
                .collect::<Vec<_>>(),
            ["second"]
        );

        // A cancelling Task accepts no answers.
        caller.cancel(task.task_id).await.unwrap();
        assert!(matches!(
            query
                .submit_input_responses(task.task_id, answers(&[("second", json!("accept"))]))
                .await,
            Err(TaskError::InvalidTransition {
                from: TaskStatus::CancelRequested,
                ..
            })
        ));
        assert!(matches!(
            worker
                .request_input(task.task_id, "third", request("Late?"))
                .await,
            Err(TaskError::InvalidTransition {
                from: TaskStatus::CancelRequested,
                ..
            })
        ));
    })
    .await;
}

#[tokio::test]
async fn concurrent_answers_accept_one() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let task = claimed(&worker, RecoveryClass::Resume).await;
        worker
            .request_input(task.task_id, "confirm", request("Proceed?"))
            .await
            .unwrap();
        let answerers = (0..8).map(|index| {
            let query = worker.for_owner(&owner());
            async move {
                query
                    .submit_input_responses(
                        task.task_id,
                        answers(&[("confirm", json!(format!("answer-{index}")))]),
                    )
                    .await
                    .unwrap()
            }
        });
        let submissions = futures::future::join_all(answerers).await;
        assert_eq!(
            submissions
                .iter()
                .map(|submission| submission.accepted)
                .sum::<usize>(),
            1
        );
    })
    .await;
}

#[tokio::test]
async fn a_task_holds_at_most_sixty_four_input_requests() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let task = claimed(&worker, RecoveryClass::Resume).await;
        // Concurrent requests for one Task still stop at the limit.
        let requests = futures::future::join_all((0..MAX_TASK_INPUTS + 8).map(|index| {
            let worker = worker.clone();
            async move {
                worker
                    .request_input(task.task_id, &format!("input-{index}"), request("Next?"))
                    .await
            }
        }))
        .await;
        let accepted = requests.iter().filter(|request| request.is_ok()).count();
        assert_eq!(accepted, MAX_TASK_INPUTS);
        assert!(requests.iter().all(|request| match request {
            Ok(_) => true,
            Err(TaskError::InputLimit(id)) => *id == task.task_id,
            Err(_) => false,
        }));
        assert_eq!(
            worker.outstanding_inputs(task.task_id).await.unwrap().len(),
            MAX_TASK_INPUTS
        );
        // Answered requests still count.
        let all = (0..MAX_TASK_INPUTS + 8)
            .map(|index| (format!("input-{index}"), json!("accept")))
            .collect::<Vec<_>>();
        let all = all
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect::<Vec<_>>();
        worker
            .submit_input_responses(task.task_id, answers(&all))
            .await
            .unwrap();
        assert!(matches!(
            worker
                .request_input(task.task_id, "after-answers", request("More?"))
                .await,
            Err(TaskError::InputLimit(_))
        ));
    })
    .await;
}

#[tokio::test]
async fn trusted_answers_apply_the_same_status_rule() {
    within(async {
        let db = TestDb::new().await;
        let worker = runtime(&db).await;
        let queued = create(&worker, RecoveryClass::Resume).await;
        assert_eq!(
            worker
                .submit_input_responses(queued.task_id, answers(&[("none", json!(1))]))
                .await
                .unwrap(),
            TaskInputSubmission {
                accepted: 0,
                ignored: 1
            }
        );
        worker.cancel(queued.task_id).await.unwrap();
        assert!(matches!(
            worker
                .submit_input_responses(queued.task_id, answers(&[("none", json!(1))]))
                .await,
            Err(TaskError::InvalidTransition {
                from: TaskStatus::Cancelled,
                ..
            })
        ));
    })
    .await;
}
