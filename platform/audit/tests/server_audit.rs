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

/// A target kind only the inventory server registers.
#[derive(Debug, Clone, PartialEq, Eq, schemars::JsonSchema)]
#[schemars(deny_unknown_fields, transform = shelf_schema)]
struct Shelf {
    shelf: u32,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum ShelfWire {
    #[serde(rename = "shelf")]
    Shelf { shelf: u32 },
}

impl serde::Serialize for Shelf {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(&ShelfWire::Shelf { shelf: self.shelf }, serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Shelf {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let ShelfWire::Shelf { shelf } = serde::Deserialize::deserialize(deserializer)?;
        Ok(Self { shelf })
    }
}

impl AuditTargetOwner for Shelf {
    const KIND: &'static str = "shelf";
}

fn shelf_schema(schema: &mut schemars::Schema) {
    let object = schema.as_object_mut().unwrap();
    object
        .get_mut("properties")
        .and_then(serde_json::Value::as_object_mut)
        .unwrap()
        .insert(
            "kind".into(),
            serde_json::json!({"type": "string", "const": "shelf"}),
        );
    object
        .get_mut("required")
        .and_then(serde_json::Value::as_array_mut)
        .unwrap()
        .push(serde_json::json!("kind"));
}

#[tokio::test]
async fn sealing_verification_and_export_need_no_owner_types() {
    within(Duration::from_secs(90), async {
        use axum::{Json, Router, extract::State, routing::post};
        use veoveo_audit::export::{AuditExportConfig, AuditExporter, OtlpConfig};
        struct Collector(tokio::task::JoinHandle<()>);
        impl Drop for Collector {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let events = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1/logs", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let app = Router::new()
            .route(
                "/v1/logs",
                post(
                    |State(events): State<Arc<std::sync::Mutex<Vec<String>>>>,
                     Json(request): Json<serde_json::Value>| async move {
                        let names = request["resourceLogs"][0]["scopeLogs"][0]["logRecords"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|log| log["eventName"].as_str().unwrap().to_owned())
                            .collect::<Vec<_>>();
                        events.lock().unwrap().extend(names);
                        ([("content-type", "application/json")], "{}")
                    },
                ),
            )
            .with_state(events.clone());
        let _collector = Collector(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        let exports = AuditExportConfig {
            s3: None,
            otlp: Some(OtlpConfig {
                endpoint,
                allow_http: true,
                bearer_token_env: None,
            }),
        };
        let destination = AuditExporter::new(exports.clone())
            .unwrap()
            .destination_ids()
            .remove(0);

        // The sealing process knows the platform's classes and `fixture.note` only.
        let audit = AuditDb::new().await;
        let mut builder = AuditRegistry::builder();
        let shelf = builder.register_target::<Shelf>().unwrap();
        builder.declare::<Moved>().unwrap();
        let server_registry = Arc::new(builder.build());
        let inventory = server_audit(&audit, "inventory", server_registry.clone()).await;
        inventory
            .declare_as_server(&slug("inventory"))
            .await
            .unwrap();
        let moved = AuditDraft::builder(
            AuditRequest::background(),
            shelf.target(&server_registry, Shelf { shelf: 4 }).unwrap(),
            server_registry.detail(&Moved { count: 3 }).unwrap(),
            AuditOutcome::Succeeded,
            AuditReason::Accepted,
        )
        .build()
        .unwrap();
        let mut transaction = inventory.platform().pool().begin().await.unwrap();
        AuditTransactionWrite::new(&server_registry, moved.clone())
            .unwrap()
            .append_as_server(&mut transaction)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        let noted = draft(note(1));
        audit.a.append(std::slice::from_ref(&noted)).await.unwrap();

        let key = Arc::new(AuditSigningKey::from_seed(&[53; 32]));
        let service = AuditService::start(
            audit.a.clone(),
            key.clone(),
            std::num::NonZeroU32::new(1).unwrap(),
            exports,
        )
        .unwrap();
        let scope = AuditReadScope::new(None, true);
        let partition = AuditPartition::Installation;
        let caught_up = tokio::time::timeout(Duration::from_secs(30), async {
            audit
                .b
                .wait_sealed(&scope, &partition, noted.id())
                .await
                .unwrap();
            while audit
                .b
                .export_candidate(&destination)
                .await
                .unwrap()
                .is_some()
            {
                assert_ne!(service.health().state(), AuditHealthState::Failed);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert_ne!(service.health().state(), AuditHealthState::Failed);
        service.shutdown(Duration::from_secs(15)).await.unwrap();
        caught_up.expect("sealing and export did not catch up within 30 seconds");
        let mut exported = events.lock().unwrap().clone();
        exported.sort();
        assert_eq!(
            exported,
            ["veoveo.audit.fixture.note", "veoveo.audit.inventory.moved"]
        );

        // Verification reads the stored records and needs no owner's types either.
        let blocks = audit.a.blocks(&scope, &partition, None, 10).await.unwrap();
        let checkpoint = audit
            .a
            .partition_checkpoint(&partition)
            .await
            .unwrap()
            .unwrap();
        let mut keys = AuditKeyRing::default();
        keys.insert(key.public_key()).unwrap();
        let mut verifier = AuditVerifier::new(
            &keys,
            partition.clone(),
            None,
            chrono::TimeDelta::minutes(1),
        )
        .unwrap();
        let mut members = Vec::new();
        for block in &blocks {
            let records = audit.a.stored_block_records(&scope, block).await.unwrap();
            assert!(verifier.verify(block, &records).unwrap().is_empty());
            members.extend(records.iter().map(|record| record.id()));
        }
        verifier.finish(&checkpoint).unwrap();
        assert_eq!(members, [moved.id(), noted.id()]);

        // Decoding the server's target and detail is a reader's concern: the sealing
        // process's registry cannot, and a reader with the server's types can, and its
        // decoded records verify against the same blocks.
        assert!(matches!(
            audit.a.block_records(&scope, &blocks[0]).await,
            Err(AuditStoreError::Target(_) | AuditStoreError::Detail(_))
        ));
        let mut builder = AuditRegistry::builder();
        builder.register_target::<Shelf>().unwrap();
        for declaration in AuditStore::declarations(&audit.db.a).await.unwrap() {
            builder.declaration(declaration).unwrap();
        }
        let reader = AuditStore::new(audit.db.a.clone(), Arc::new(builder.build()));
        let mut verifier =
            AuditVerifier::new(&keys, partition, None, chrono::TimeDelta::minutes(1)).unwrap();
        for block in &blocks {
            let records = reader.block_records(&scope, block).await.unwrap();
            assert!(verifier.verify(block, &records).unwrap().is_empty());
        }
        verifier.finish(&checkpoint).unwrap();
    })
    .await;
}
