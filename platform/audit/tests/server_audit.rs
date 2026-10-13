//! A registered server audits through its own login: it declares only the classes under
//! its prefix, appends only records of those classes, inside its own transaction, and
//! its records take the same commit order and sealed positions as every other record.
mod support;

use std::{sync::Arc, time::Duration};
use support::{AuditDb, archive, draft, note, seal_all, within};
use veoveo_audit::{
    integrity::{AuditKeyRing, AuditSigningKey, AuditVerifier},
    sealer::AuditSealer,
    *,
};
use veoveo_types::ServerSlug;

/// The inventory server's detail, class `inventory.moved`.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
struct Moved {
    count: u32,
}

impl AuditDetailType for Moved {
    const CLASS: &'static str = "inventory.moved";
}

/// The same class with another schema, as a later version of the server declares it.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
struct MovedWithBin {
    count: u32,
    bin: String,
}

impl AuditDetailType for MovedWithBin {
    const CLASS: &'static str = "inventory.moved";
}

/// The stockroom server's detail, class `stockroom.binned`.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
struct Binned {
    count: u32,
}

impl AuditDetailType for Binned {
    const CLASS: &'static str = "stockroom.binned";
}

/// A class under the inventory prefix that nobody declares.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
struct Counted {
    count: u32,
}

impl AuditDetailType for Counted {
    const CLASS: &'static str = "inventory.counted";
}

/// `fixture.note` with another schema, which a server named `fixture` tries to declare
/// over the installation owner's declaration.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
struct NoteTakeover {
    text: String,
}

impl AuditDetailType for NoteTakeover {
    const CLASS: &'static str = "fixture.note";
}

fn slug(value: &str) -> ServerSlug {
    ServerSlug::parse(value).unwrap()
}

/// The platform's classes, the fixture's `fixture.note` and both servers' declared
/// classes.
fn servers_registry() -> Arc<AuditRegistry> {
    let mut builder = AuditRegistry::builder();
    support::audit_owner::declare_audit_detail(&mut builder).unwrap();
    builder.declare::<Moved>().unwrap();
    builder.declare::<Binned>().unwrap();
    Arc::new(builder.build())
}

fn registry_of(declare: impl FnOnce(&mut AuditRegistryBuilder)) -> Arc<AuditRegistry> {
    let mut builder = AuditRegistry::builder();
    declare(&mut builder);
    Arc::new(builder.build())
}

/// An audit store connected through a fresh login of `server`.
async fn server_audit(audit: &AuditDb, server: &str, registry: Arc<AuditRegistry>) -> AuditStore {
    AuditStore::new(audit.db.server_store(&slug(server)).await, registry)
}

fn record<T: AuditDetailType>(registry: &AuditRegistry, detail: &T) -> AuditDraft {
    draft(registry.detail(detail).unwrap())
}

async fn declaring_server(audit: &AuditDb, class: &str) -> Option<String> {
    sqlx::query_file_scalar!("tests/queries/server/declaring_server.sql", class)
        .fetch_one(audit.db.a.pool())
        .await
        .unwrap()
}

async fn declared_schema(audit: &AuditDb, class: &str) -> serde_json::Value {
    sqlx::query_file_scalar!("tests/queries/server/declared_schema.sql", class)
        .fetch_one(audit.db.a.pool())
        .await
        .unwrap()
}

/// The owner's view of the log, with every class any owner declared in the database, as
/// a reader of other owners' records builds its registry.
async fn reader(audit: &AuditDb) -> AuditStore {
    let mut builder = AuditRegistry::builder();
    for declaration in AuditStore::declarations(&audit.db.a).await.unwrap() {
        builder.declaration(declaration).unwrap();
    }
    AuditStore::new(audit.db.a.clone(), Arc::new(builder.build()))
}

fn all() -> (AuditReadScope, AuditQuery) {
    let mut query = AuditQuery::new(AuditPartition::Installation);
    query.limit = 1000;
    (AuditReadScope::new(None, true), query)
}

#[tokio::test]
async fn a_server_declares_only_the_classes_under_its_own_prefix() {
    within(Duration::from_secs(90), async {
        let audit = AuditDb::new().await;
        let registry = servers_registry();
        let inventory = server_audit(&audit, "inventory", registry.clone()).await;
        let stockroom = server_audit(&audit, "stockroom", registry.clone()).await;

        inventory
            .declare_as_server(&slug("inventory"))
            .await
            .unwrap();
        assert_eq!(
            declaring_server(&audit, "inventory.moved").await.as_deref(),
            Some("inventory")
        );
        // Repeating a declaration changes nothing.
        inventory
            .declare_as_server(&slug("inventory"))
            .await
            .unwrap();

        // The inventory login holds no stockroom role, so stockroom's prefix is refused.
        assert!(matches!(
            inventory.declare_as_server(&slug("stockroom")).await,
            Err(AuditStoreError::ForeignClass)
        ));
        stockroom
            .declare_as_server(&slug("stockroom"))
            .await
            .unwrap();
        assert_eq!(
            declaring_server(&audit, "stockroom.binned")
                .await
                .as_deref(),
            Some("stockroom")
        );

        // A server whose slug matches the prefix of a class the installation owner
        // declared cannot replace it.
        let takeover = server_audit(
            &audit,
            "fixture",
            registry_of(|builder| builder.declare::<NoteTakeover>().unwrap()),
        )
        .await;
        let before = declared_schema(&audit, "fixture.note").await;
        assert!(matches!(
            takeover.declare_as_server(&slug("fixture")).await,
            Err(AuditStoreError::ForeignClass)
        ));
        assert_eq!(declaring_server(&audit, "fixture.note").await, None);
        assert_eq!(declared_schema(&audit, "fixture.note").await, before);

        // A later version of the server replaces its own schema.
        let upgraded = server_audit(
            &audit,
            "inventory",
            registry_of(|builder| builder.declare::<MovedWithBin>().unwrap()),
        )
        .await;
        upgraded
            .declare_as_server(&slug("inventory"))
            .await
            .unwrap();
        let schema = declared_schema(&audit, "inventory.moved").await;
        assert!(schema["properties"].get("bin").is_some(), "{schema}");
        assert_eq!(
            declaring_server(&audit, "inventory.moved").await.as_deref(),
            Some("inventory")
        );
    })
    .await;
}

#[tokio::test]
async fn a_server_appends_only_its_own_records_inside_its_own_transaction() {
    within(Duration::from_secs(90), async {
        let audit = AuditDb::new().await;
        let registry = servers_registry();
        let inventory = server_audit(&audit, "inventory", registry.clone()).await;
        let stockroom = server_audit(&audit, "stockroom", registry.clone()).await;
        inventory
            .declare_as_server(&slug("inventory"))
            .await
            .unwrap();
        stockroom
            .declare_as_server(&slug("stockroom"))
            .await
            .unwrap();
        let pool = inventory.platform().pool();
        sqlx::query(include_str!("queries/server/domain_table.sql"))
            .execute(pool)
            .await
            .unwrap();
        let moves = || async {
            sqlx::query_scalar::<_, i64>(include_str!("queries/server/domain_count.sql"))
                .fetch_one(pool)
                .await
                .unwrap()
        };
        let (scope, query) = all();
        let owner = reader(&audit).await;

        // The server's own record commits with its own write.
        let committed = record(&registry, &Moved { count: 1 });
        let mut transaction = pool.begin().await.unwrap();
        sqlx::query(include_str!("queries/server/domain_insert.sql"))
            .bind(1)
            .execute(&mut *transaction)
            .await
            .unwrap();
        AuditTransactionWrite::new(&registry, committed.clone())
            .unwrap()
            .append_as_server(&mut transaction)
            .await
            .unwrap();
        assert!(
            audit
                .a
                .page(&scope, &query)
                .await
                .unwrap()
                .records
                .is_empty(),
            "the owner sees nothing before the server commits"
        );
        transaction.commit().await.unwrap();
        assert_eq!(moves().await, 1);
        let stored = owner.page(&scope, &query).await.unwrap().records;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].draft, committed);

        // It rolls back with the server's transaction.
        let mut transaction = pool.begin().await.unwrap();
        sqlx::query(include_str!("queries/server/domain_insert.sql"))
            .bind(2)
            .execute(&mut *transaction)
            .await
            .unwrap();
        AuditTransactionWrite::new(&registry, record(&registry, &Moved { count: 2 }))
            .unwrap()
            .append_as_server(&mut transaction)
            .await
            .unwrap();
        transaction.rollback().await.unwrap();
        assert_eq!(moves().await, 1);
        assert_eq!(owner.page(&scope, &query).await.unwrap().records.len(), 1);

        // Records of a class the installation owner or another server declared are
        // refused, and the refusal aborts the server's transaction.
        for foreign in [record(&registry, &Binned { count: 1 }), draft(note(1))] {
            let mut transaction = pool.begin().await.unwrap();
            assert!(matches!(
                AuditTransactionWrite::new(&registry, foreign)
                    .unwrap()
                    .append_as_server(&mut transaction)
                    .await,
                Err(AuditAppendError::ForeignClass)
            ));
            assert!(
                sqlx::query(include_str!("queries/server/domain_insert.sql"))
                    .bind(3)
                    .execute(&mut *transaction)
                    .await
                    .is_err(),
                "a refused append aborts the caller's transaction"
            );
            transaction.rollback().await.unwrap();
        }
        // A class under its own prefix that it never declared fails like any
        // undeclared class.
        let undeclared = registry_of(|builder| builder.declare::<Counted>().unwrap());
        let mut transaction = pool.begin().await.unwrap();
        assert!(matches!(
            AuditTransactionWrite::new(&undeclared, record(&undeclared, &Counted { count: 1 }))
                .unwrap()
                .append_as_server(&mut transaction)
                .await,
            Err(AuditAppendError::UndeclaredClass)
        ));
        transaction.rollback().await.unwrap();
        assert_eq!(moves().await, 1);
        assert_eq!(owner.page(&scope, &query).await.unwrap().records.len(), 1);

        // The login reaches the audit tables only through the two server functions.
        assert!(
            sqlx::query_scalar::<_, i64>(include_str!("queries/server/read_records.sql"))
                .fetch_one(pool)
                .await
                .is_err(),
            "a server login cannot read audit records"
        );

        // The owner's own path still appends its classes, and refuses nothing it did
        // before.
        audit.a.append(&[draft(note(2))]).await.unwrap();
        assert_eq!(owner.page(&scope, &query).await.unwrap().records.len(), 2);
    })
    .await;
}

#[tokio::test]
async fn server_records_take_commit_order_and_seal_without_gaps() {
    within(Duration::from_secs(90), async {
        let audit = AuditDb::new().await;
        let registry = servers_registry();
        let inventory = server_audit(&audit, "inventory", registry.clone()).await;
        inventory
            .declare_as_server(&slug("inventory"))
            .await
            .unwrap();
        let owner = reader(&audit).await;
        let key = Arc::new(AuditSigningKey::from_seed(&[7; 32]));
        let sealer = AuditSealer::acquire(owner.clone(), key.clone())
            .await
            .unwrap();
        let partition = AuditPartition::Installation;
        let scope = AuditReadScope::new(None, true);

        // The server's transaction takes its ID before the owner writes, then appends
        // after the owner committed. Its record still precedes the owner's.
        let first = record(&registry, &Moved { count: 1 });
        let second = draft(note(1));
        let mut transaction = inventory.platform().pool().begin().await.unwrap();
        sqlx::query_file_scalar!("tests/queries/sealing/assign_txid.sql")
            .fetch_one(&mut *transaction)
            .await
            .unwrap();
        audit.a.append(std::slice::from_ref(&second)).await.unwrap();
        AuditTransactionWrite::new(&registry, first.clone())
            .unwrap()
            .append_as_server(&mut transaction)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        let third = record(&registry, &Moved { count: 2 });
        let mut transaction = inventory.platform().pool().begin().await.unwrap();
        AuditTransactionWrite::new(&registry, third.clone())
            .unwrap()
            .append_as_server(&mut transaction)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        assert_eq!(seal_all(&sealer).await, 3);

        let archived = archive(&owner, &scope, &partition).await;
        let positions = archived
            .iter()
            .flat_map(|(block, _)| &block.head.members)
            .map(|member| (member.id, member.position.get()))
            .collect::<Vec<_>>();
        assert_eq!(
            positions,
            [(first.id(), 1), (second.id(), 2), (third.id(), 3)]
        );
        let checkpoint = owner
            .partition_checkpoint(&partition)
            .await
            .unwrap()
            .unwrap();
        let mut keys = AuditKeyRing::default();
        keys.insert(key.public_key()).unwrap();
        let mut verifier =
            AuditVerifier::new(&keys, partition, None, chrono::TimeDelta::minutes(1)).unwrap();
        for (block, records) in &archived {
            assert!(verifier.verify(block, records).unwrap().is_empty());
        }
        verifier.finish(&checkpoint).unwrap();
        sealer.release().await.unwrap();
    })
    .await;
}
