//! The store fixture's extension hook against the PostgreSQL on `PATH`, which must carry
//! pgvector 0.8.7, as the development image installs it.
use veoveo_fixtures::store::TestDb;
use veoveo_types::ServerSlug;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_extensions_schema_and_vector_are_created_as_the_installation_bootstrap_creates_them() {
    let db = TestDb::new().await;
    db.create_extensions(&["vector"]).await;
    // Creating them again changes nothing.
    db.create_extensions(&["vector"]).await;

    // The superuser owns the schema; the installation owner may use it but not write it.
    let (owner, usage, create): (String, bool, bool) = sqlx::query_as(
        "SELECT r.rolname,
            pg_catalog.has_schema_privilege('veoveo_owner', n.oid, 'USAGE'),
            pg_catalog.has_schema_privilege('veoveo_owner', n.oid, 'CREATE')
         FROM pg_catalog.pg_namespace AS n
         JOIN pg_catalog.pg_roles AS r ON r.oid = n.nspowner
         WHERE n.nspname = '_extensions'",
    )
    .fetch_one(db.a.pool())
    .await
    .unwrap();
    assert_eq!(owner, "postgres");
    assert!(usage && !create);
    let (version, schema, extension_owner): (String, String, String) = sqlx::query_as(
        "SELECT e.extversion, n.nspname, r.rolname
         FROM pg_catalog.pg_extension AS e
         JOIN pg_catalog.pg_namespace AS n ON n.oid = e.extnamespace
         JOIN pg_catalog.pg_roles AS r ON r.oid = e.extowner
         WHERE e.extname = 'vector'",
    )
    .fetch_one(db.a.pool())
    .await
    .unwrap();
    assert_eq!(
        (version.as_str(), schema.as_str(), extension_owner.as_str()),
        ("0.8.7", "_extensions", "postgres")
    );

    // A server's login uses the type and its operator from its own schema.
    let store = db
        .server_store(&ServerSlug::parse("fixture").unwrap())
        .await;
    sqlx::raw_sql(
        "CREATE TABLE fixture.item (embedding _extensions.vector(3));
         INSERT INTO fixture.item VALUES ('[1,0,0]');",
    )
    .execute(store.pool())
    .await
    .unwrap();
    let distance: f64 = sqlx::query_scalar(
        "SELECT embedding OPERATOR(_extensions.<=>) '[0,1,0]'::_extensions.vector FROM fixture.item",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(distance, 1.0);
    store.close().await;
}

#[tokio::test]
#[should_panic(expected = "is not an extension name")]
async fn a_name_that_is_not_an_extensions_is_refused() {
    TestDb::new()
        .await
        .create_extensions(&["vector\" CASCADE; --"])
        .await;
}

#[tokio::test]
#[should_panic(expected = "Store fixture extension: absent_extension")]
async fn an_extension_the_postgresql_on_path_lacks_fails_the_test() {
    TestDb::new()
        .await
        .create_extensions(&["absent_extension"])
        .await;
}
