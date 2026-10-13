//! Class declarations in `audit.detail_type`.
use super::{AuditStore, AuditStoreError};
use veoveo_audit_contract::{AuditClass, AuditDetailDeclaration};
use veoveo_platform_store::PlatformStore;

impl AuditStore {
    /// Record the registry's declarations, the platform's included. A record's class must
    /// be declared, so every owner declares before its first append. Repeating a
    /// declaration changes nothing, and a changed schema replaces the stored one.
    pub async fn declare(&self) -> Result<(), AuditStoreError> {
        let (classes, schemas): (Vec<AuditClass>, Vec<serde_json::Value>) = self
            .registry()
            .declarations()
            .map(|declaration| (declaration.class().clone(), declaration.schema().clone()))
            .unzip();
        sqlx::query_file!(
            "src/store/declarations/declare.sql",
            &classes as &[AuditClass],
            &schemas,
        )
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Every declaration owners stored. A reader adds them to its registry to read every
    /// owner's records.
    pub async fn declarations(
        store: &PlatformStore,
    ) -> Result<Vec<AuditDetailDeclaration>, AuditStoreError> {
        let rows = sqlx::query_file!("src/store/declarations/declarations.sql")
            .fetch_all(store.pool())
            .await?;
        rows.into_iter()
            .map(|row| {
                AuditDetailDeclaration::new(row.class, row.schema).map_err(AuditStoreError::from)
            })
            .collect()
    }
}
