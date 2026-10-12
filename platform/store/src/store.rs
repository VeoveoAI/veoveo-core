use std::fmt;

use sqlx::{
    ConnectOptions, PgConnection, PgPool,
    migrate::Migrator,
    postgres::{PgConnectOptions, PgPoolOptions},
};

use crate::{StoreConfig, StoreError};

/// The kernel migrations in `platform/store/migrations`. Their history lives in
/// `kernel._sqlx_migrations`, as this crate's `sqlx.toml` declares.
pub static MIGRATOR: Migrator = sqlx::migrate!();

/// A connection pool to the installation database and the settings for the dedicated
/// connections that instances, singletons and listeners hold.
#[derive(Clone)]
pub struct PlatformStore {
    pool: PgPool,
    options: PgConnectOptions,
    config: StoreConfig,
}

impl PlatformStore {
    /// Open the pool and check that the database accepts the login.
    pub async fn connect(config: StoreConfig) -> Result<Self, StoreError> {
        let options = config.connect_options();
        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections())
            .acquire_timeout(config.acquire_timeout())
            .idle_timeout(config.idle_timeout())
            .max_lifetime(config.max_lifetime())
            .connect_with(options.clone())
            .await?;
        Ok(Self {
            pool,
            options,
            config,
        })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn config(&self) -> &StoreConfig {
        &self.config
    }

    /// Apply the kernel migrations. Concurrent replicas wait on the migrator's advisory
    /// lock, and a changed checksum of an applied migration fails.
    pub async fn migrate(&self) -> Result<(), StoreError> {
        MIGRATOR.run(&self.pool).await?;
        Ok(())
    }

    /// Close the pool after its connections are returned.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// A connection outside the pool, for state that lives as long as its session.
    pub(crate) async fn dedicated_connection(&self) -> Result<PgConnection, StoreError> {
        let timeout = self.config.acquire_timeout();
        tokio::time::timeout(timeout, self.options.connect())
            .await
            .map_err(|_| StoreError::Timeout {
                operation: "opening a dedicated database connection",
                seconds: timeout.as_secs(),
            })?
            .map_err(StoreError::from)
    }

    pub(crate) fn connect_options(&self) -> &PgConnectOptions {
        &self.options
    }
}

impl fmt::Debug for PlatformStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlatformStore")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}
