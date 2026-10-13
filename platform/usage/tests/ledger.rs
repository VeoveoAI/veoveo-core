//! The usage ledger against a fresh PostgreSQL database per test: confined declarations
//! and appends, idempotent and conflicting repeats, Task costs, daily totals, concurrent
//! repeats, money rules, row-level security and monthly partitions.
#[allow(dead_code, reason = "each test binary uses part of the shared fixture")]
#[path = "../../../testing/fixtures/store.rs"]
mod store;

use std::{future::Future, time::Duration};

use chrono::{Datelike, TimeDelta, Utc};
use store::TestDb;
use veoveo_types::{PrincipalId, ServerSlug, TaskId, TenantId, WorkContextId};
use veoveo_usage::{
    Amount, ChargePeriod, Currency, Money, Quantity, ServiceCategory, UsageActor, UsageDraft,
    UsageKind, UsageKindDeclaration, UsagePhase, UsageSourceId, UsageStore, UsageStoreError,
    UsageUnit, UsageWrite,
};

async fn within<T>(case: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(60), case)
        .await
        .expect("test case exceeded 60 seconds")
}

fn kind(value: &str) -> UsageKind {
    UsageKind::parse(value).unwrap()
}

fn declaration(name: &str, unit: UsageUnit, monetary: bool) -> UsageKindDeclaration {
    UsageKindDeclaration {
        kind: kind(name),
        service_category: ServiceCategory::AiAndMachineLearning,
        unit,
        monetary,
    }
}

fn actor(principal: &str) -> UsageActor {
    UsageActor {
        tenant: TenantId::parse("acme").unwrap(),
        principal: PrincipalId::parse(principal).unwrap(),
        work_context: WorkContextId::parse("studio").unwrap(),
    }
}

fn usd(amount: &str) -> Money {
    Money {
        amount: Amount::parse(amount).unwrap(),
        currency: Currency::parse("USD").unwrap(),
    }
}

fn draft(
    name: &str,
    phase: UsagePhase,
    source: &str,
    quantity: u64,
    money: Option<Money>,
    task: Option<TaskId>,
) -> UsageDraft {
    let mut builder = UsageDraft::builder(
        kind(name),
        phase,
        actor("pilot"),
        Quantity::whole(quantity),
        UsageSourceId::parse(source).unwrap(),
        ChargePeriod::at(Utc::now()),
    );
    if let Some(money) = money {
        builder = builder.money(money);
    }
    if let Some(task) = task {
        builder = builder.task(task, Some("generate"));
    }
    builder.build().unwrap()
}

async fn media(db: &TestDb) -> UsageStore {
    let store = UsageStore::new(db.server_store(&ServerSlug::parse("media").unwrap()).await);
    store
        .declare_as_server(&[
            declaration("media.generation", UsageUnit::units(), true),
            declaration("media.tokens", UsageUnit::tokens(), false),
        ])
        .await
        .unwrap();
    store
}

#[tokio::test]
async fn a_server_declares_and_appends_only_its_own_kinds() {
    within(async {
        let db = TestDb::new().await;
        let owner = UsageStore::new(db.a.clone());
        owner
            .declare(&[declaration("reason.tokens", UsageUnit::tokens(), false)])
            .await
            .unwrap();
        let media = media(&db).await;
        // Repeating a declaration changes nothing.
        media
            .declare_as_server(&[declaration("media.generation", UsageUnit::units(), true)])
            .await
            .unwrap();
        assert!(matches!(
            media
                .declare_as_server(&[declaration("reason.tokens", UsageUnit::tokens(), false)])
                .await,
            Err(UsageStoreError::ForeignKind(_))
        ));
        assert!(matches!(
            media
                .append_as_server(&[draft(
                    "reason.tokens",
                    UsagePhase::Actual,
                    "r-1",
                    5,
                    None,
                    None
                )])
                .await,
            Err(UsageStoreError::ForeignKind(_))
        ));
        assert_eq!(
            media
                .append_as_server(&[draft(
                    "media.tokens",
                    UsagePhase::Actual,
                    "t-1",
                    5,
                    None,
                    None
                )])
                .await
                .unwrap(),
            1
        );
    })
    .await;
}

#[tokio::test]
async fn a_repeat_writes_nothing_and_a_changed_repeat_conflicts() {
    within(async {
        let db = TestDb::new().await;
        let media = media(&db).await;
        let estimate = draft(
            "media.generation",
            UsagePhase::Estimate,
            "task:1:generate",
            1,
            Some(usd("0.04")),
            None,
        );
        assert_eq!(
            media
                .append_as_server(std::slice::from_ref(&estimate))
                .await
                .unwrap(),
            1
        );
        let repeat = draft(
            "media.generation",
            UsagePhase::Estimate,
            "task:1:generate",
            1,
            Some(usd("0.04")),
            None,
        );
        assert_eq!(media.append_as_server(&[repeat]).await.unwrap(), 0);
        let changed = draft(
            "media.generation",
            UsagePhase::Estimate,
            "task:1:generate",
            1,
            Some(usd("0.05")),
            None,
        );
        assert!(matches!(
            media.append_as_server(&[changed]).await,
            Err(UsageStoreError::Conflict(_))
        ));
        // The same source identifier in the other phase is its own record.
        let actual = draft(
            "media.generation",
            UsagePhase::Actual,
            "task:1:generate",
            1,
            Some(usd("0.03")),
            None,
        );
        assert_eq!(media.append_as_server(&[actual]).await.unwrap(), 1);
    })
    .await;
}

#[tokio::test]
async fn a_tasks_cost_is_its_actuals_else_its_estimates() {
    within(async {
        let db = TestDb::new().await;
        let media = media(&db).await;
        let task = TaskId::new();
        media
            .append_as_server(&[
                draft(
                    "media.generation",
                    UsagePhase::Estimate,
                    "g-e",
                    2,
                    Some(usd("0.08")),
                    Some(task),
                ),
                draft(
                    "media.generation",
                    UsagePhase::Actual,
                    "g-a1",
                    1,
                    Some(usd("0.03")),
                    Some(task),
                ),
                draft(
                    "media.generation",
                    UsagePhase::Actual,
                    "g-a2",
                    1,
                    Some(usd("0.035")),
                    Some(task),
                ),
                draft(
                    "media.tokens",
                    UsagePhase::Estimate,
                    "t-e",
                    120,
                    None,
                    Some(task),
                ),
            ])
            .await
            .unwrap();
        let cost = media
            .task_usage(&TenantId::parse("acme").unwrap(), task)
            .await
            .unwrap();
        assert_eq!(cost.len(), 2, "{cost:?}");
        assert_eq!(cost[0].kind, kind("media.generation"));
        assert_eq!(cost[0].phase, UsagePhase::Actual);
        assert_eq!(cost[0].quantity, Quantity::parse("2").unwrap());
        assert_eq!(cost[0].money, Some(usd("0.065")));
        assert_eq!(cost[1].kind, kind("media.tokens"));
        assert_eq!(cost[1].phase, UsagePhase::Estimate);
        assert_eq!(cost[1].unit, UsageUnit::tokens());
        assert_eq!(cost[1].money, None);
        // Another tenant's read finds nothing.
        assert!(
            media
                .task_usage(&TenantId::parse("other").unwrap(), task)
                .await
                .unwrap()
                .is_empty()
        );
    })
    .await;
}

#[tokio::test]
async fn daily_totals_follow_every_append_in_its_transaction() {
    within(async {
        let db = TestDb::new().await;
        let media = media(&db).await;
        media
            .append_as_server(&[
                draft(
                    "media.generation",
                    UsagePhase::Actual,
                    "d-1",
                    1,
                    Some(usd("0.03")),
                    None,
                ),
                draft(
                    "media.generation",
                    UsagePhase::Actual,
                    "d-2",
                    3,
                    Some(usd("0.09")),
                    None,
                ),
            ])
            .await
            .unwrap();
        // A rolled-back append leaves the totals as they were.
        let login = db.server_store(&ServerSlug::parse("media").unwrap()).await;
        let mut transaction = login.pool().begin().await.unwrap();
        UsageWrite::append_as_server(
            &mut transaction,
            &[draft(
                "media.generation",
                UsagePhase::Actual,
                "d-3",
                7,
                Some(usd("1")),
                None,
            )],
        )
        .await
        .unwrap();
        transaction.rollback().await.unwrap();
        login.close().await;

        let today = Utc::now().date_naive();
        let days = UsageStore::new(db.a.clone())
            .daily(
                &TenantId::parse("acme").unwrap(),
                today,
                today + TimeDelta::days(1),
                100,
            )
            .await
            .unwrap();
        assert_eq!(days.len(), 1, "{days:?}");
        assert_eq!(days[0].records, 2);
        assert_eq!(days[0].quantity, Quantity::parse("4").unwrap());
        assert_eq!(days[0].money, Some(usd("0.12")));
    })
    .await;
}

#[tokio::test]
async fn concurrent_repeats_store_one_record() {
    within(async {
        let db = TestDb::new().await;
        let media = media(&db).await;
        let writes = (0..8).map(|_| {
            let media = media.clone();
            async move {
                media
                    .append_as_server(&[draft(
                        "media.generation",
                        UsagePhase::Actual,
                        "race",
                        1,
                        Some(usd("0.03")),
                        None,
                    )])
                    .await
                    .unwrap()
            }
        });
        let added: u64 = futures_sum(writes).await;
        assert_eq!(added, 1);
    })
    .await;
}

async fn futures_sum(
    writes: impl Iterator<Item = impl Future<Output = u64> + Send + 'static>,
) -> u64 {
    let handles: Vec<_> = writes.map(tokio::spawn).collect();
    let mut total = 0;
    for handle in handles {
        total += handle.await.unwrap();
    }
    total
}

#[tokio::test]
async fn money_follows_the_kind() {
    within(async {
        let db = TestDb::new().await;
        let media = media(&db).await;
        assert!(matches!(
            media
                .append_as_server(&[draft(
                    "media.generation",
                    UsagePhase::Actual,
                    "m-1",
                    1,
                    None,
                    None
                )])
                .await,
            Err(UsageStoreError::Refused(_))
        ));
        assert!(matches!(
            media
                .append_as_server(&[draft(
                    "media.tokens",
                    UsagePhase::Actual,
                    "m-2",
                    1,
                    Some(usd("1")),
                    None
                )])
                .await,
            Err(UsageStoreError::Refused(_))
        ));
    })
    .await;
}

#[tokio::test]
async fn a_server_reads_only_its_own_usage() {
    within(async {
        let db = TestDb::new().await;
        let media = media(&db).await;
        let reason = UsageStore::new(db.server_store(&ServerSlug::parse("reason").unwrap()).await);
        reason
            .declare_as_server(&[declaration("reason.tokens", UsageUnit::tokens(), false)])
            .await
            .unwrap();
        let task = TaskId::new();
        reason
            .append_as_server(&[draft(
                "reason.tokens",
                UsagePhase::Actual,
                "r",
                9,
                None,
                Some(task),
            )])
            .await
            .unwrap();
        let tenant = TenantId::parse("acme").unwrap();
        assert_eq!(reason.task_usage(&tenant, task).await.unwrap().len(), 1);
        assert!(media.task_usage(&tenant, task).await.unwrap().is_empty());
        // The installation owner reads every server's usage.
        assert_eq!(
            UsageStore::new(db.a.clone())
                .task_usage(&tenant, task)
                .await
                .unwrap()
                .len(),
            1
        );
    })
    .await;
}

#[tokio::test]
async fn records_land_in_their_month_once_its_partition_exists() {
    within(async {
        let db = TestDb::new().await;
        let owner = UsageStore::new(db.a.clone());
        let today = Utc::now().date_naive();
        let through = today.checked_add_months(chrono::Months::new(2)).unwrap();
        assert_eq!(owner.prepare_partitions(through).await.unwrap(), 3);
        assert_eq!(owner.prepare_partitions(through).await.unwrap(), 0);
        let media = media(&db).await;
        media
            .append_as_server(&[draft(
                "media.tokens",
                UsagePhase::Actual,
                "p-1",
                3,
                None,
                None,
            )])
            .await
            .unwrap();
        let partition = sqlx::query_file_scalar!("tests/queries/record_partition.sql", "p-1")
            .fetch_one(db.a.pool())
            .await
            .unwrap();
        assert_eq!(
            partition,
            format!("usage.record_{}_{:02}", today.year(), today.month())
        );
    })
    .await;
}
