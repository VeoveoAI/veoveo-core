//! Singleton roles: one replica holds a session advisory lock and performs the role's
//! writes on the connection that holds it; the others wait on the same lock.
use std::fmt;

use sqlx::{Connection, PgConnection};

use crate::{PlatformStore, SingletonName, StoreError};

/// The lock for one singleton role and the connection that holds it. Postgres grants
/// the lock to the next waiting replica when this connection ends.
pub struct Singleton {
    name: SingletonName,
    connection: PgConnection,
}

impl Singleton {
    /// Wait, as a standby while another replica holds the role, until this process holds
    /// it. The wait has no deadline.
    pub async fn acquire(store: &PlatformStore, name: SingletonName) -> Result<Self, StoreError> {
        let mut connection = store.dedicated_connection().await?;
        sqlx::query_file!("src/singleton/lock_singleton.sql", name.as_str())
            .execute(&mut connection)
            .await?;
        Ok(Self { name, connection })
    }

    pub fn name(&self) -> &SingletonName {
        &self.name
    }

    /// The connection that holds the lock. The role's writes go through it, so a write
    /// fails once the lock is gone.
    pub fn connection(&mut self) -> &mut PgConnection {
        &mut self.connection
    }

    /// Give up the role by closing the lock connection.
    pub async fn release(self) -> Result<(), StoreError> {
        self.connection.close().await?;
        Ok(())
    }
}

impl fmt::Debug for Singleton {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Singleton")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}
