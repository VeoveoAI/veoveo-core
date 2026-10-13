//! Caller selection in SQL and row-level security between servers.
use veoveo_task_runtime::{Instance, RecoveryClass, TaskError, TaskRuntime, TaskStatus};
use veoveo_types::{TaskId, TaskTypeName, WorkContextId};

use crate::{
    store::TestDb,
    support::{OwnerSpec, TASK_TYPE, draft, owner, owner_with, runtime, runtime_for, slug, within},
};

#[tokio::test]
async fn owner_queries_select_the_callers_tasks() {
    within(async {
        let db = TestDb::new().await;
        let runtime = runtime(&db).await;
        let labelled = owner_with(OwnerSpec {
            labels: &["secret"],
            ..OwnerSpec::default()
        });
        let task = runtime
            .create(draft(&runtime, &labelled, RecoveryClass::Resume))
            .await
            .unwrap()
            .snapshot;

        // The owner holding at least the Task's labels reads it.
        let cleared = owner_with(OwnerSpec {
            labels: &["secret", "other"],
            ..OwnerSpec::default()
        });
        assert_eq!(
            runtime.for_owner(&cleared).get(task.task_id).await.unwrap(),
            Some(task.clone())
        );
        // Every other selection reads nothing, exactly like a missing Task.
        for denied in [
            owner(),
            owner_with(OwnerSpec {
                principal: "copilot",
                labels: &["secret"],
                ..OwnerSpec::default()
            }),
            owner_with(OwnerSpec {
                profile: "auditor",
                labels: &["secret"],
                ..OwnerSpec::default()
            }),
            owner_with(OwnerSpec {
                tenant: Some("other-tenant"),
                labels: &["secret"],
                ..OwnerSpec::default()
            }),
            owner_with(OwnerSpec {
                tenant: None,
                labels: &["secret"],
                ..OwnerSpec::default()
            }),
        ] {
            let query = runtime.for_owner(&denied);
            assert_eq!(query.get(task.task_id).await.unwrap(), None);
            assert!(matches!(
                query.cancel(task.task_id).await,
                Err(TaskError::NotFound(_))
            ));
        }
        assert_eq!(
            runtime
                .for_owner(&cleared)
                .get(TaskId::new())
                .await
                .unwrap(),
            None
        );

        // Operation and Work Context selections narrow the rows in SQL.
        let other_type = TaskTypeName::new("time.validate_timeline").unwrap();
        assert_eq!(
            runtime
                .for_owner(&cleared)
                .of_type(other_type.clone())
                .get(task.task_id)
                .await
                .unwrap(),
            None
        );
        assert!(
            runtime
                .for_owner(&cleared)
                .of_types([other_type, TASK_TYPE])
                .unwrap()
                .get(task.task_id)
                .await
                .unwrap()
                .is_some()
        );
        assert!(matches!(
            runtime.for_owner(&cleared).of_types([]),
            Err(TaskError::InvalidPageQuery)
        ));
        assert!(
            runtime
                .for_owner(&cleared)
                .in_work_context()
                .unwrap()
                .get(task.task_id)
                .await
                .unwrap()
                .is_some()
        );
        let mut elsewhere = cleared.clone();
        elsewhere.authority.work_context = WorkContextId::parse("elsewhere").unwrap();
        assert_eq!(
            runtime
                .for_owner(&elsewhere)
                .in_work_context()
                .unwrap()
                .get(task.task_id)
                .await
                .unwrap(),
            None
        );
        // Without the Work Context selection, the context does not narrow the rows.
        assert!(
            runtime
                .for_owner(&elsewhere)
                .get(task.task_id)
                .await
                .unwrap()
                .is_some()
        );

        // The owner cancels its own Task.
        let cancelled = runtime
            .for_owner(&cleared)
            .cancel(task.task_id)
            .await
            .unwrap();
        assert_eq!(cancelled.status, TaskStatus::Cancelled);
    })
    .await;
}

#[tokio::test]
async fn pages_follow_creation_order_and_reapply_the_selection() {
    within(async {
        let db = TestDb::new().await;
        let runtime = runtime(&db).await;
        let mut created = Vec::new();
        for _ in 0..5 {
            created.push(
                runtime
                    .create(draft(&runtime, &owner(), RecoveryClass::Resume))
                    .await
                    .unwrap()
                    .snapshot
                    .task_id,
            );
        }
        let stranger = owner_with(OwnerSpec {
            principal: "stranger",
            ..OwnerSpec::default()
        });
        runtime
            .create(draft(&runtime, &stranger, RecoveryClass::Resume))
            .await
            .unwrap();

        let query = runtime.for_owner(&owner());
        let first = query.page(None, 2).await.unwrap();
        let cursor = first.next_cursor.clone().expect("more Tasks follow");
        let second = query.page(Some(&cursor), 2).await.unwrap();
        let third = query.page(second.next_cursor.as_ref(), 2).await.unwrap();
        assert!(third.next_cursor.is_none());
        let paged = first
            .items
            .iter()
            .chain(&second.items)
            .chain(&third.items)
            .map(|task| task.task_id)
            .collect::<Vec<_>>();
        assert_eq!(paged, created);

        // A cursor is a position; another caller reading after it sees only its own Tasks.
        let foreign = runtime
            .for_owner(&stranger)
            .page(Some(&cursor), 10)
            .await
            .unwrap();
        assert_eq!(foreign.items.len(), 1);
        assert!(foreign.items.iter().all(|task| task.owner == stranger));
        for limit in [0, 1001] {
            assert!(matches!(
                query.page(None, limit).await,
                Err(TaskError::InvalidPageQuery)
            ));
        }
    })
    .await;
}

#[tokio::test]
async fn row_level_security_confines_a_server_login_to_its_own_tasks() {
    within(async {
        let db = TestDb::new().await;
        let alpha = db.server_store(&slug("alpha")).await;
        let beta = db.server_store(&slug("beta")).await;
        let alpha_runtime = runtime_for(&alpha, "alpha").await;
        let beta_runtime = runtime_for(&beta, "beta").await;
        // A login cannot start an instance for another server.
        assert!(Instance::start(&alpha, &slug("beta")).await.is_err());
        let alpha_task = alpha_runtime
            .create(draft(&alpha_runtime, &owner(), RecoveryClass::Resume))
            .await
            .unwrap()
            .snapshot;
        let beta_task = beta_runtime
            .create(draft(&beta_runtime, &owner(), RecoveryClass::Resume))
            .await
            .unwrap()
            .snapshot;

        // A runtime built for another server reads nothing through the policy.
        let intruder = TaskRuntime::new(
            alpha.clone(),
            slug("beta"),
            alpha_runtime.instance().clone(),
        );
        assert!(intruder.get(beta_task.task_id).await.unwrap().is_none());
        assert!(intruder.claim_next().await.unwrap().is_none());
        assert!(matches!(
            intruder.cancel(beta_task.task_id).await,
            Err(TaskError::NotFound(_))
        ));
        assert!(
            intruder
                .create(draft(&intruder, &owner(), RecoveryClass::Resume))
                .await
                .is_err()
        );
        assert_eq!(
            intruder
                .prune_expired(veoveo_task_runtime::MaintenanceBatch::DEFAULT)
                .await
                .unwrap()
                .rows,
            0
        );

        // Each login still runs its own work, and the owner sees both.
        assert_eq!(
            alpha_runtime
                .claim_next()
                .await
                .unwrap()
                .map(|task| task.task_id),
            Some(alpha_task.task_id)
        );
        let owner_view = runtime_for(&db.a, "beta").await;
        assert!(owner_view.get(beta_task.task_id).await.unwrap().is_some());
        assert!(
            beta_runtime
                .get(alpha_task.task_id)
                .await
                .unwrap()
                .is_none()
        );
    })
    .await;
}

/// The index conditions of every node of an `EXPLAIN (FORMAT JSON)` plan.
fn index_conditions(node: &serde_json::Value, conditions: &mut Vec<String>) {
    if let Some(condition) = node.get("Index Cond").and_then(serde_json::Value::as_str) {
        conditions.push(condition.to_owned());
    }
    for child in node
        .get("Plans")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        index_conditions(child, conditions);
    }
}

#[tokio::test]
async fn a_later_page_seeks_its_cursor_in_a_generic_plan() {
    within(async {
        let db = TestDb::new().await;
        let script = include_str!("../queries/plans/owner_page.sql");
        assert!(
            script.contains(include_str!("../../queries/owner/page.sql").trim_end()),
            "the plan script prepares the page statement verbatim"
        );
        let mut connection = db.a.pool().acquire().await.unwrap();
        let rows = sqlx::raw_sql(script)
            .fetch_all(&mut *connection)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let plan: serde_json::Value = sqlx::Row::try_get(&rows[0], 0).unwrap();
        let mut conditions = Vec::new();
        index_conditions(&plan[0]["Plan"], &mut conditions);
        assert!(
            conditions
                .iter()
                .any(|condition| condition.contains("ROW(created_at, id) > ROW(COALESCE(")),
            "{conditions:?}"
        );
    })
    .await;
}
