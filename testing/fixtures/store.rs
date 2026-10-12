//! A fresh PostgreSQL database per test, cloned from a migrated template on a private
//! cluster. Requires the PostgreSQL 18 server programs on `PATH`.
use std::{sync::Arc, time::Duration};

use secrecy::SecretString;
use sqlx::migrate::Migrator;
use uuid::Uuid;
use veoveo_platform_store::{PlatformStore, ServerSchema, StoreConfig};
use veoveo_types::ServerSlug;

#[path = "store/cluster.rs"]
mod cluster;

pub use cluster::Failure;
use cluster::{Cluster, OWNER, random_secret};

const SETUP: Duration = Duration::from_secs(120);

/// One database owned by this test. `a` and `b` are two independent pools connected as
/// the installation owner, as two Gateway replicas would be. Dropping the fixture drops
/// the database; the cluster stops when the last fixture of the process is dropped.
pub struct TestDb {
    pub a: PlatformStore,
    pub b: PlatformStore,
    database: String,
    cluster: Arc<Cluster>,
}

impl TestDb {
    pub async fn new() -> Self {
        tokio::time::timeout(SETUP, Self::create())
            .await
            .unwrap_or_else(|_| panic!("Store fixture setup exceeded {SETUP:?}"))
            .unwrap_or_else(|failure| panic!("{failure}"))
    }

    async fn create() -> Result<Self, Failure> {
        let cluster = Cluster::shared().await?;
        let database = format!("t_{}", Uuid::new_v4().simple());
        cluster.create_database(&database).await?;
        let config = cluster.owner_config(&database);
        let a = connect(config.clone(), "first owner pool").await?;
        let b = connect(config, "second owner pool").await?;
        Ok(Self {
            a,
            b,
            database,
            cluster,
        })
    }

    /// The database URL for a service the test starts, without a password; the user is
    /// the installation owner.
    pub fn url(&self) -> String {
        format!(
            "postgres://{OWNER}@127.0.0.1:{}/{}?sslmode=disable",
            self.cluster.port(),
            self.database
        )
    }

    /// The installation owner's password for [`Self::url`].
    pub fn password(&self) -> &SecretString {
        self.cluster.owner_password()
    }

    /// The owner's connection settings for this database.
    pub fn config(&self) -> StoreConfig {
        self.cluster.owner_config(&self.database)
    }

    pub fn database(&self) -> &str {
        &self.database
    }

    /// Another owner pool, such as a third replica.
    pub async fn connect(&self) -> PlatformStore {
        connect(self.config(), "owner pool")
            .await
            .unwrap_or_else(|failure| panic!("{failure}"))
    }

    /// Register `server` as the Gateway would: its group role and schema, and a login
    /// `<schema>_<random>` with a random password. Returns a pool connected as that
    /// login, which row-level security and grants treat as the server.
    pub async fn server_store(&self, server: &ServerSlug) -> PlatformStore {
        let setup = async {
            let schema =
                ServerSchema::new(server).map_err(|error| Failure::new("server name", error))?;
            self.a
                .ensure_server(server)
                .await
                .map_err(|error| Failure::new("server schema", error))?;
            let login = schema
                .login(&Uuid::new_v4().simple().to_string()[..8])
                .map_err(|error| Failure::new("server login", error))?;
            let password = random_secret();
            self.a
                .create_server_login(server, &login, &password)
                .await
                .map_err(|error| Failure::new("server login", error))?;
            connect(
                self.config().with_login(login.as_str(), password),
                "server pool",
            )
            .await
        };
        tokio::time::timeout(SETUP, setup)
            .await
            .unwrap_or_else(|_| panic!("Store fixture server login exceeded {SETUP:?}"))
            .unwrap_or_else(|failure| panic!("{failure}"))
    }

    /// Apply a server's migrations in its schema as the server does at startup: as one
    /// of its logins, after taking its group role.
    pub async fn migrate_server(&self, server: &ServerSlug, migrator: Migrator) {
        let store = self.server_store(server).await;
        let migrated = tokio::time::timeout(SETUP, store.migrate_server(server, migrator)).await;
        store.close().await;
        migrated
            .unwrap_or_else(|_| panic!("Store fixture server migration exceeded {SETUP:?}"))
            .unwrap_or_else(|error| panic!("Store fixture server migration: {error}"));
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        // The last fixture removes the whole cluster instead.
        if Arc::strong_count(&self.cluster) > 1
            && let Err(failure) = self.cluster.drop_database(&self.database)
        {
            eprintln!("{failure}");
        }
    }
}

async fn connect(config: StoreConfig, stage: &'static str) -> Result<PlatformStore, Failure> {
    tokio::time::timeout(Duration::from_secs(30), PlatformStore::connect(config))
        .await
        .map_err(|_| Failure::new(stage, "connection exceeded 30 seconds"))?
        .map_err(|error| Failure::new(stage, error))
}
