//! Caller selection in SQL and row-level security between servers.
use veoveo_task::{
    Instance, MAX_GET_MANY, OwnerTaskQuery, RecoveryClass, TaskError, TaskPageOrder, TaskRuntime,
    TaskStatus,
};
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
                surface: "auditor",
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
        let oldest = TaskPageOrder::OldestFirst;
        let first = query.page(oldest, None, 2).await.unwrap();
        let cursor = first.next_cursor.clone().expect("more Tasks follow");
        let second = query.page(oldest, Some(&cursor), 2).await.unwrap();
        let third = query
            .page(oldest, second.next_cursor.as_ref(), 2)
            .await
            .unwrap();
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
            .page(oldest, Some(&cursor), 10)
            .await
            .unwrap();
        assert_eq!(foreign.items.len(), 1);
        assert!(foreign.items.iter().all(|task| task.owner == stranger));
        for limit in [0, 1001] {
            assert!(matches!(
                query.page(oldest, None, limit).await,
                Err(TaskError::InvalidPageQuery)
            ));
        }
    })
    .await;
}

/// Every Task the query selects, reading pages of `limit` in `order` until the last.
async fn every_page(query: &OwnerTaskQuery, order: TaskPageOrder, limit: usize) -> Vec<TaskId> {
    let mut tasks = Vec::new();
    let mut cursor = None;
    loop {
        let page = query.page(order, cursor.as_ref(), limit).await.unwrap();
        tasks.extend(page.items.iter().map(|task| task.task_id));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return tasks,
        }
    }
}

/// Give `tasks` one creation time. The revision trigger keeps a Task's creation time, so
/// the test turns it off around the update in one transaction.
async fn same_creation_time(db: &TestDb, tasks: &[TaskId]) {
    let ids = tasks.iter().map(|task| task.as_uuid()).collect::<Vec<_>>();
    let mut transaction = db.a.pool().begin().await.unwrap();
    sqlx::raw_sql("ALTER TABLE tasks.task DISABLE TRIGGER task_revision")
        .execute(&mut *transaction)
        .await
        .unwrap();
    sqlx::query_file!("tests/queries/same_creation_time.sql", &ids)
        .execute(&mut *transaction)
        .await
        .unwrap();
    sqlx::raw_sql("ALTER TABLE tasks.task ENABLE TRIGGER task_revision")
        .execute(&mut *transaction)
        .await
        .unwrap();
    transaction.commit().await.unwrap();
}

#[tokio::test]
async fn pages_read_oldest_or_newest_first_and_keep_the_selection() {
    within(async {
        let db = TestDb::new().await;
        let runtime = runtime(&db).await;
        let mut mine = Vec::new();
        for _ in 0..5 {
            mine.push(
                runtime
                    .create(draft(&runtime, &owner(), RecoveryClass::Resume))
                    .await
                    .unwrap()
                    .snapshot
                    .task_id,
            );
        }
        // Three Tasks share one creation time, so their IDs order them within a page
        // and across a page boundary.
        same_creation_time(&db, &mine[1..4]).await;
        let mut other = draft(&runtime, &owner(), RecoveryClass::Resume);
        other.task_type = TaskTypeName::new("time.validate_timeline").unwrap();
        let other_type = runtime.create(other).await.unwrap().snapshot.task_id;
        let elsewhere = runtime
            .create(draft(
                &runtime,
                &owner_with(OwnerSpec {
                    work_context: "elsewhere",
                    ..OwnerSpec::default()
                }),
                RecoveryClass::Resume,
            ))
            .await
            .unwrap()
            .snapshot
            .task_id;
        // A Task with a label the caller lacks is never selected.
        runtime
            .create(draft(
                &runtime,
                &owner_with(OwnerSpec {
                    labels: &["secret"],
                    ..OwnerSpec::default()
                }),
                RecoveryClass::Resume,
            ))
            .await
            .unwrap();

        let mut visible = mine.clone();
        visible.extend([other_type, elsewhere]);
        let mut snapshots = runtime.get_many(&visible).await.unwrap();
        snapshots.sort_by_key(|task| (task.created_at, task.task_id.as_uuid()));
        let ordered = |keep: &dyn Fn(&TaskId) -> bool, order: TaskPageOrder| {
            let mut tasks = snapshots
                .iter()
                .map(|task| task.task_id)
                .filter(|task| keep(task))
                .collect::<Vec<_>>();
            if order == TaskPageOrder::NewestFirst {
                tasks.reverse();
            }
            tasks
        };
        let all = runtime.for_owner(&owner());
        let typed = runtime.for_owner(&owner()).of_type(TASK_TYPE);
        let in_context = runtime.for_owner(&owner()).in_work_context().unwrap();
        for order in [TaskPageOrder::OldestFirst, TaskPageOrder::NewestFirst] {
            for limit in [1, 2, 3, 10] {
                assert_eq!(
                    every_page(&all, order, limit).await,
                    ordered(&|_| true, order),
                    "{order} by {limit}"
                );
            }
            assert_eq!(
                every_page(&typed, order, 2).await,
                ordered(&|task| *task != other_type, order),
            );
            assert_eq!(
                every_page(&in_context, order, 2).await,
                ordered(&|task| *task != elsewhere, order),
            );
        }

        // A cursor continues only the order that issued it.
        let newest = all
            .page(TaskPageOrder::NewestFirst, None, 2)
            .await
            .unwrap()
            .next_cursor
            .expect("more Tasks follow");
        assert_eq!(newest.order(), TaskPageOrder::NewestFirst);
        assert_eq!(
            serde_json::to_value(&newest).unwrap()["order"],
            "newest_first"
        );
        assert!(matches!(
            all.page(TaskPageOrder::OldestFirst, Some(&newest), 2).await,
            Err(TaskError::PageOrderMismatch {
                cursor: TaskPageOrder::NewestFirst,
                requested: TaskPageOrder::OldestFirst,
            })
        ));
        let oldest = all
            .page(TaskPageOrder::OldestFirst, None, 2)
            .await
            .unwrap()
            .next_cursor
            .expect("more Tasks follow");
        assert!(matches!(
            all.page(TaskPageOrder::NewestFirst, Some(&oldest), 2).await,
            Err(TaskError::PageOrderMismatch { .. })
        ));
    })
    .await;
}

#[tokio::test]
async fn a_batch_reads_only_the_servers_own_tasks() {
    within(async {
        let db = TestDb::new().await;
        let alpha = db.server_store(&slug("alpha")).await;
        let beta = db.server_store(&slug("beta")).await;
        let alpha_runtime = runtime_for(&alpha, "alpha").await;
        let beta_runtime = runtime_for(&beta, "beta").await;
        let mut alpha_tasks = Vec::new();
        for _ in 0..2 {
            alpha_tasks.push(
                alpha_runtime
                    .create(draft(&alpha_runtime, &owner(), RecoveryClass::Resume))
                    .await
                    .unwrap()
                    .snapshot
                    .task_id,
            );
        }
        let beta_task = beta_runtime
            .create(draft(&beta_runtime, &owner(), RecoveryClass::Resume))
            .await
            .unwrap()
            .snapshot
            .task_id;
        alpha_tasks.sort_by_key(|task| task.as_uuid());

        // One query returns the server's Tasks in ID order; another server's Task, a
        // missing ID and a repeated ID add nothing.
        let read = alpha_runtime
            .get_many(&[
                alpha_tasks[1],
                beta_task,
                TaskId::new(),
                alpha_tasks[0],
                alpha_tasks[1],
            ])
            .await
            .unwrap();
        assert_eq!(
            read.iter().map(|task| task.task_id).collect::<Vec<_>>(),
            alpha_tasks
        );
        assert_eq!(beta_runtime.get_many(&alpha_tasks).await.unwrap().len(), 0);
        assert!(alpha_runtime.get_many(&[]).await.unwrap().is_empty());

        // A runtime named for another server reads nothing through the login's policy.
        let intruder = TaskRuntime::new(
            alpha.clone(),
            slug("beta"),
            alpha_runtime.instance().clone(),
        );
        assert!(intruder.get_many(&[beta_task]).await.unwrap().is_empty());

        // A batch past the limit is refused before any read.
        let oversized = (0..=MAX_GET_MANY)
            .map(|_| TaskId::new())
            .collect::<Vec<_>>();
        assert!(matches!(
            alpha_runtime.get_many(&oversized).await,
            Err(TaskError::BatchTooLarge {
                requested,
                max: MAX_GET_MANY,
            }) if requested == MAX_GET_MANY + 1
        ));
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
                .prune_expired(veoveo_task::MaintenanceBatch::DEFAULT)
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

/// The index scans of every node of an `EXPLAIN (FORMAT JSON)` plan, as index name, scan
/// direction and index condition.
fn index_scans(node: &serde_json::Value, scans: &mut Vec<(String, String, String)>) {
    if let Some(index) = node.get("Index Name").and_then(serde_json::Value::as_str) {
        let field = |name: &str| {
            node.get(name)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        scans.push((
            index.to_owned(),
            field("Scan Direction"),
            field("Index Cond"),
        ));
    }
    for child in node
        .get("Plans")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        index_scans(child, scans);
    }
}

#[tokio::test]
async fn a_later_newest_first_page_scans_the_owner_index_backward() {
    within(async {
        let db = TestDb::new().await;
        let script = include_str!("../queries/plans/owner_page_newest.sql");
        assert!(
            script.contains(include_str!("../../queries/owner/page_newest.sql").trim_end()),
            "the plan script prepares the newest-first page statement verbatim"
        );
        let mut connection = db.a.pool().acquire().await.unwrap();
        let rows = sqlx::raw_sql(script)
            .fetch_all(&mut *connection)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let plan: serde_json::Value = sqlx::Row::try_get(&rows[0], 0).unwrap();
        let mut scans = Vec::new();
        index_scans(&plan[0]["Plan"], &mut scans);
        assert!(
            scans.iter().any(|(index, direction, condition)| {
                index == "task_owner_page"
                    && direction == "Backward"
                    && condition.contains("ROW(created_at, id) < ROW(COALESCE(")
            }),
            "{scans:?}"
        );
    })
    .await;
}
