//! Registered servers' roles and schemas, and the migrations a server applies in its own
//! schema.
use secrecy::{ExposeSecret, SecretString};
use sqlx::{Connection, migrate::Migrator};
use veoveo_types::ServerSlug;

use crate::{LoginName, PlatformStore, ServerSchema, StoreError};

impl PlatformStore {
    /// Create the server's group role and schema. The Gateway calls it when a server
    /// registers; repeating it changes nothing. Requires the installation owner.
    pub async fn ensure_server(&self, server: &ServerSlug) -> Result<ServerSchema, StoreError> {
        let schema = ServerSchema::new(server)?;
        sqlx::query_file!("src/servers/ensure_server.sql", server.as_str())
            .execute(self.pool())
            .await?;
        Ok(schema)
    }

    /// Create the login for one registration of `server`, a member of the server's group
    /// role. Requires the installation owner.
    pub async fn create_server_login(
        &self,
        server: &ServerSlug,
        login: &LoginName,
        password: &SecretString,
    ) -> Result<(), StoreError> {
        ServerSchema::new(server)?;
        sqlx::query_file!(
            "src/servers/create_server_login.sql",
            server.as_str(),
            login.as_str(),
            password.expose_secret(),
        )
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Drop a login that [`Self::create_server_login`] created. Dropping an absent login
    /// does nothing. Requires the installation owner.
    pub async fn drop_server_login(&self, login: &LoginName) -> Result<(), StoreError> {
        sqlx::query_file!("src/servers/drop_server_login.sql", login.as_str())
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// The slugs of the servers whose group role this login belongs to.
    pub async fn current_servers(&self) -> Result<Vec<String>, StoreError> {
        Ok(sqlx::query_file_scalar!("src/servers/current_servers.sql")
            .fetch_one(self.pool())
            .await?)
    }

    /// Apply a server's migrations in its own schema, on a dedicated connection that has
    /// taken the server's group role, so the group role owns every object and the
    /// history lives in `<schema>._sqlx_migrations`. Unqualified names resolve in the
    /// server's schema. The migrator must not create schemas; the Gateway has created
    /// the server's schema with [`Self::ensure_server`].
    pub async fn migrate_server(
        &self,
        server: &ServerSlug,
        mut migrator: Migrator,
    ) -> Result<(), StoreError> {
        let schema = ServerSchema::new(server)?;
        migrator.dangerous_set_table_name(schema.migrations_table());
        let mut connection = self.dedicated_connection().await?;
        let migrated = async {
            sqlx::query_file_scalar!("src/servers/set_role.sql", schema.group_role())
                .fetch_one(&mut connection)
                .await?;
            sqlx::query_file_scalar!("src/servers/set_search_path.sql", schema.schema())
                .fetch_one(&mut connection)
                .await?;
            migrator.run(&mut connection).await?;
            Ok::<_, StoreError>(())
        }
        .await;
        let closed = connection.close().await;
        migrated?;
        closed?;
        Ok(())
    }
}
