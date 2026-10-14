//! Kernel behavior against a private PostgreSQL 18 cluster: migrations, server roles and
//! schemas, process instances, singletons and change notifications.
#[allow(dead_code, reason = "each test binary uses part of the shared fixture")]
#[path = "../../../testing/fixtures/store.rs"]
mod store;

use std::time::Duration;

use futures::StreamExt;
use sqlx::migrate::Migrator;
use store::TestDb;
use veoveo_platform_store::{
    Change, ChangeOperation, ChangeSignal, ChangeStream, Instance, InstanceId, InstancePhase,
    InstanceState, LoginName, PlatformStore, ServerSchema, Singleton, SingletonName, StoreError,
    TenantId,
};
use veoveo_types::ServerSlug;

fn slug(value: &str) -> ServerSlug {
    ServerSlug::parse(value).unwrap()
}

async fn fixture_migrator() -> Migrator {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture_server");
    Migrator::new(directory).await.unwrap()
}

async fn visible_instances(store: &PlatformStore) -> Vec<(InstanceId, String, String)> {
    sqlx::query_file!("tests/queries/kernel/visible_instances.sql")
        .fetch_all(store.pool())
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.id, row.server, row.login))
        .collect()
}

async fn terminate(store: &PlatformStore, pid: i32) {
    let terminated = sqlx::query_file_scalar!("tests/queries/kernel/terminate_backend.sql", pid)
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert!(terminated);
}

async fn within<T>(limit: Duration, future: impl Future<Output = T>) -> T {
    tokio::time::timeout(limit, future)
        .await
        .unwrap_or_else(|_| panic!("exceeded {limit:?}"))
}

async fn next_signal(stream: &mut ChangeStream) -> ChangeSignal {
    within(Duration::from_secs(20), stream.next())
        .await
        .expect("the change stream continues")
        .expect("the change stream delivers a signal")
}

#[tokio::test]
async fn kernel_migrations_are_recorded_in_the_kernel_schema_and_repeat_as_no_ops() {
    let db = TestDb::new().await;
    let recorded = sqlx::query_file_scalar!("tests/queries/kernel/history_table.sql")
        .fetch_one(db.a.pool())
        .await
        .unwrap();
    assert!(recorded);
    db.a.migrate().await.unwrap();
    db.b.migrate().await.unwrap();
}

#[tokio::test]
async fn instances_hold_their_lock_until_shutdown_and_are_reaped_once_it_is_gone() {
    let db = TestDb::new().await;
    let gateway = slug("gateway");
    let running = Instance::start(&db.a, &gateway).await.unwrap();
    let abandoned = Instance::start(&db.b, &gateway).await.unwrap();
    assert!(db.a.instance_alive(running.id()).await.unwrap());
    assert!(db.a.instance_alive(abandoned.id()).await.unwrap());
    assert_eq!(db.a.reap_instances().await.unwrap(), 0);

    let abandoned_id = abandoned.id();
    drop(abandoned);
    within(Duration::from_secs(10), async {
        while db.a.instance_alive(abandoned_id).await.unwrap() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert_eq!(db.a.reap_instances().await.unwrap(), 1);
    let rows = visible_instances(&db.a).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, running.id());
    assert_eq!(rows[0].1, "gateway");
    assert_eq!(rows[0].2, db.config().username());

    let mut reaps = running.reaps();
    let running_id = running.id();
    within(
        Duration::from_secs(10),
        reaps.wait_for(|round| round.round >= 1),
    )
    .await
    .unwrap();
    running.shutdown().await.unwrap();
    assert!(!db.a.instance_alive(running_id).await.unwrap());
    assert!(visible_instances(&db.a).await.is_empty());
}

#[tokio::test]
async fn a_shared_instance_shuts_down_while_other_holders_keep_it() {
    let db = TestDb::new().await;
    let shared = std::sync::Arc::new(Instance::start(&db.a, &slug("gateway")).await.unwrap());
    let holder = shared.clone();
    let id = shared.id();
    shared.shutdown().await.unwrap();
    assert!(holder.is_lost());
    assert!(!db.a.instance_alive(id).await.unwrap());
    assert!(visible_instances(&db.a).await.is_empty());
    holder.shutdown().await.unwrap();
}

/// The session that holds an instance's lock, if any.
async fn lock_backend(store: &PlatformStore, id: InstanceId) -> Option<i32> {
    sqlx::query_file_scalar!(
        "tests/queries/kernel/instance_backend.sql",
        id as InstanceId
    )
    .fetch_optional(store.pool())
    .await
    .unwrap()
}

/// Wait until the instance is live on a session other than `previous`.
async fn live_again(instance: &Instance, observer: &PlatformStore, previous: i32) -> InstanceState {
    within(Duration::from_secs(30), async {
        loop {
            let state = instance.state();
            if state.phase == InstancePhase::Live
                && lock_backend(observer, state.id)
                    .await
                    .is_some_and(|pid| pid != previous)
            {
                return state;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
}

#[tokio::test]
async fn an_instance_whose_connection_ends_takes_its_lock_back() {
    let db = TestDb::new().await;
    let instance = Instance::start(&db.a, &slug("gateway")).await.unwrap();
    let id = instance.id();
    let pid = lock_backend(&db.b, id)
        .await
        .expect("the instance holds its lock");
    terminate(&db.b, pid).await;

    // The instance notices within a check and holds the same id again, so the work it
    // owns stays its own.
    let state = live_again(&instance, &db.b, pid).await;
    assert_eq!(state.id, id);
    assert_eq!(state.generation, 0);
    assert!(!instance.is_lost());
    assert!(db.b.instance_alive(id).await.unwrap());
    assert_eq!(db.b.reap_instances().await.unwrap(), 0);
    assert_eq!(visible_instances(&db.b).await.len(), 1);
    instance.shutdown().await.unwrap();
    assert!(visible_instances(&db.b).await.is_empty());
}

#[tokio::test]
async fn an_instance_whose_row_was_reaped_continues_as_a_new_one() {
    let db = TestDb::new().await;
    let clock = db.server_store(&slug("clock")).await;
    let instance = Instance::start(&clock, &slug("clock")).await.unwrap();
    let first = instance.id();
    let pid = lock_backend(&db.b, first)
        .await
        .expect("the instance holds its lock");
    // A reaper that found the lock free while the connection was down deleted the row.
    sqlx::query_file!(
        "tests/queries/kernel/delete_instance.sql",
        first as InstanceId
    )
    .execute(db.b.pool())
    .await
    .unwrap();

    let state = live_again(&instance, &db.b, pid).await;
    assert_ne!(state.id, first);
    assert_eq!(state.generation, 1);
    assert!(!instance.is_lost());
    assert!(!db.b.instance_alive(first).await.unwrap());
    assert!(clock.instance_alive(state.id).await.unwrap());
    let seen = visible_instances(&clock).await;
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, state.id);
    instance.shutdown().await.unwrap();
    assert!(visible_instances(&clock).await.is_empty());
}

#[tokio::test]
async fn a_resume_ends_the_earlier_session_of_its_login_and_refuses_other_logins() {
    let db = TestDb::new().await;
    let clock = db.server_store(&slug("clock")).await;
    let instance = Instance::start(&clock, &slug("clock")).await.unwrap();
    let id = instance.id();
    let pid = lock_backend(&db.b, id)
        .await
        .expect("the instance holds its lock");

    // Another login cannot take the instance.
    let resume = |store: PlatformStore| async move {
        sqlx::query_file_scalar!("tests/queries/kernel/resume_instance.sql", id as InstanceId)
            .fetch_one(store.pool())
            .await
            .unwrap()
    };
    assert_eq!(resume(db.b.clone()).await, "gone");
    assert_eq!(lock_backend(&db.b, id).await, Some(pid));

    // The same login finds the lock held, ends the session that holds it and leaves the
    // lock to the instance, which takes it back.
    assert_eq!(resume(clock.clone()).await, "held");
    let state = live_again(&instance, &db.b, pid).await;
    assert_eq!(state.id, id);
    assert_eq!(state.generation, 0);
    instance.shutdown().await.unwrap();
    assert_eq!(resume(clock.clone()).await, "gone");
}

#[tokio::test]
async fn an_instance_whose_login_was_dropped_ends() {
    let db = TestDb::new().await;
    let clock = db.server_store(&slug("clock")).await;
    let instance = Instance::start(&clock, &slug("clock")).await.unwrap();
    // The Gateway removed the registration and dropped its login, and the sessions of
    // the login ended, the instance's lock session among them.
    db.remove_login(&clock).await;

    within(Duration::from_secs(30), instance.lost()).await;
    assert!(instance.is_lost());
    assert_eq!(instance.state().phase, InstancePhase::Ended);
    assert!(matches!(
        instance.shutdown().await,
        Err(StoreError::InstanceLost)
    ));
    assert_eq!(db.b.reap_instances().await.unwrap(), 1);
}

#[tokio::test]
async fn server_logins_see_and_start_only_their_own_instances() {
    let db = TestDb::new().await;
    let clock = db.server_store(&slug("clock")).await;
    let calendar = db.server_store(&slug("calendar")).await;
    assert_eq!(clock.current_servers().await.unwrap(), vec!["clock"]);
    assert!(db.a.current_servers().await.unwrap().is_empty());

    let clock_instance = Instance::start(&clock, &slug("clock")).await.unwrap();
    let calendar_instance = Instance::start(&calendar, &slug("calendar")).await.unwrap();
    assert!(matches!(
        Instance::start(&clock, &slug("calendar")).await,
        Err(StoreError::Database(_))
    ));
    let seen = visible_instances(&clock).await;
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, clock_instance.id());
    assert_eq!(seen[0].2, clock.config().username());
    assert_eq!(visible_instances(&db.a).await.len(), 2);

    // A server cannot manage roles or read identity tables.
    assert!(clock.ensure_server(&slug("calendar")).await.is_err());
    assert!(
        clock
            .work_context_by_key(TenantId::from_uuid(uuid::Uuid::nil()), "development")
            .await
            .is_err()
    );
    clock_instance.shutdown().await.unwrap();
    calendar_instance.shutdown().await.unwrap();
}

#[tokio::test]
async fn server_names_and_logins_are_checked_in_rust_and_sql() {
    let db = TestDb::new().await;
    for name in ["tasks", "pg-temp", "time_zones", "1time"] {
        assert!(matches!(
            db.a.ensure_server(&slug(name)).await,
            Err(StoreError::InvalidName { .. })
        ));
    }
    let clock = slug("clock");
    db.a.ensure_server(&clock).await.unwrap();
    db.a.ensure_server(&clock).await.unwrap();
    let password = secrecy::SecretString::from("a-password-of-sufficient-length".to_owned());
    let foreign = LoginName::parse("calendar_0b6f").unwrap();
    assert!(
        db.a.create_server_login(&clock, &foreign, &password)
            .await
            .is_err()
    );
    let short = secrecy::SecretString::from("short".to_owned());
    let login = ServerSchema::new(&clock).unwrap().login("0b6f").unwrap();
    assert!(
        db.a.create_server_login(&clock, &login, &short)
            .await
            .is_err()
    );
    db.a.create_server_login(&clock, &login, &password)
        .await
        .unwrap();
    let store = PlatformStore::connect(db.config().with_login(login.as_str(), password))
        .await
        .unwrap();
    store.close().await;
    db.a.drop_server_login(&login).await.unwrap();
    db.a.drop_server_login(&login).await.unwrap();
    let owner = LoginName::parse(db.config().username()).unwrap();
    assert!(db.a.drop_server_login(&owner).await.is_err());
}

#[tokio::test]
async fn servers_migrate_in_their_own_schema_as_their_group_role() {
    let db = TestDb::new().await;
    let server = slug("fixture-server");
    db.migrate_server(&server, fixture_migrator().await).await;
    db.migrate_server(&server, fixture_migrator().await).await;
    for table in ["note", "_sqlx_migrations"] {
        let owner = sqlx::query_file_scalar!("tests/queries/fixture_server/note_owner.sql", table)
            .fetch_one(db.a.pool())
            .await
            .unwrap();
        assert_eq!(owner.as_deref(), Some("server_fixture_server"), "{table}");
    }
}

#[tokio::test]
async fn singletons_admit_one_holder_and_pass_to_the_waiting_replica() {
    let db = TestDb::new().await;
    let name = SingletonName::parse("fixture.sealer").unwrap();
    let mut holder = Singleton::acquire(&db.a, name.clone()).await.unwrap();
    sqlx::query_file_scalar!("tests/queries/kernel/history_table.sql")
        .fetch_one(holder.connection())
        .await
        .unwrap();
    let standby = tokio::spawn({
        let store = db.b.clone();
        let name = name.clone();
        async move { Singleton::acquire(&store, name).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!standby.is_finished());
    holder.release().await.unwrap();
    let successor = within(Duration::from_secs(10), standby)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(successor.name(), &name);
    successor.release().await.unwrap();
}

#[tokio::test]
async fn listeners_reconcile_at_start_and_after_reconnecting_and_wake_on_changes() {
    let db = TestDb::new().await;
    let server = slug("fixture-server");
    db.migrate_server(&server, fixture_migrator().await).await;
    let writer = db.server_store(&server).await;
    let channel = ServerSchema::new(&server).unwrap().channel();
    let mut stream = db.a.listen(vec![channel.clone()]);
    assert_eq!(next_signal(&mut stream).await, ChangeSignal::Reconcile);

    let tenant = uuid::Uuid::new_v4();
    let note = uuid::Uuid::new_v4();
    sqlx::query(include_str!("queries/fixture_server/insert_note.sql"))
        .bind(note)
        .bind(tenant)
        .bind("first")
        .execute(writer.pool())
        .await
        .unwrap();
    assert_eq!(
        next_signal(&mut stream).await,
        ChangeSignal::Changed(Change {
            channel: channel.clone(),
            table: "note".to_owned(),
            operation: ChangeOperation::Insert,
            key: Some(note.to_string()),
            tenant: Some(TenantId::from_uuid(tenant)),
        })
    );

    let listeners = sqlx::query_file_scalar!("tests/queries/kernel/listener_backends.sql")
        .fetch_all(db.b.pool())
        .await
        .unwrap();
    assert_eq!(listeners.len(), 1);
    terminate(&db.b, listeners[0]).await;
    // The terminated session may first surface as an error; the stream then subscribes
    // again and asks for a reconcile.
    within(Duration::from_secs(30), async {
        loop {
            match stream.next().await.expect("the change stream continues") {
                Ok(ChangeSignal::Reconcile) => break,
                Ok(ChangeSignal::Changed(change)) => panic!("unexpected {change:?}"),
                Err(_) => continue,
            }
        }
    })
    .await;

    let second = uuid::Uuid::new_v4();
    sqlx::query(include_str!("queries/fixture_server/insert_note.sql"))
        .bind(second)
        .bind(None::<uuid::Uuid>)
        .bind("second")
        .execute(writer.pool())
        .await
        .unwrap();
    let ChangeSignal::Changed(change) = next_signal(&mut stream).await else {
        panic!("expected a change after reconnecting");
    };
    assert_eq!(change.key, Some(second.to_string()));
    assert_eq!(change.tenant, None);
}
