//! Class declarations in `audit.detail_type`.
use super::{AuditStore, AuditStoreError};
use veoveo_audit_contract::{AuditClass, AuditDetailDeclaration, AuditOcsfClass};
use veoveo_platform_store::PlatformStore;
use veoveo_types::ServerSlug;

impl AuditStore {
    /// Record the registry's declarations, the platform's included. A record's class must
    /// be declared, so every owner declares before its first append. Repeating a
    /// declaration changes nothing, and a changed schema replaces the stored one.
    pub async fn declare(&self) -> Result<(), AuditStoreError> {
        let columns = columns(self.registry().declarations());
        sqlx::query_file!(
            "src/store/declarations/declare.sql",
            &columns.classes as &[AuditClass],
            &columns.schemas as &[Option<serde_json::Value>],
            &columns.ocsf_classes,
            &columns.ocsf_activities,
        )
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Record the declarations of `server`'s own classes, those under the `<server>.`
    /// prefix, through a store connected as one of that server's logins. The database
    /// refuses a class outside the prefix of the login's server, or one the installation
    /// owner or another server declared, with [`AuditStoreError::ForeignClass`].
    /// Repeating a declaration changes nothing, and a changed schema replaces the
    /// server's own one.
    pub async fn declare_as_server(&self, server: &ServerSlug) -> Result<(), AuditStoreError> {
        let columns = columns(
            self.registry()
                .declarations()
                .filter(|declaration| declaration.class().owner() == server.as_str()),
        );
        sqlx::query_file!(
            "src/store/declarations/declare_server.sql",
            &columns.classes as &[AuditClass],
            &columns.schemas as &[Option<serde_json::Value>],
            &columns.ocsf_classes,
            &columns.ocsf_activities,
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
                let ocsf = AuditOcsfClass {
                    class_uid: u32::try_from(row.ocsf_class_uid)
                        .map_err(|_| AuditStoreError::Integrity)?,
                    activity_id: u8::try_from(row.ocsf_activity_id)
                        .map_err(|_| AuditStoreError::Integrity)?,
                };
                AuditDetailDeclaration::new(row.class, ocsf, row.schema)
                    .map_err(AuditStoreError::from)
            })
            .collect()
    }
}

/// Declarations as the declaring statements bind them.
struct Columns {
    classes: Vec<AuditClass>,
    schemas: Vec<Option<serde_json::Value>>,
    ocsf_classes: Vec<i32>,
    ocsf_activities: Vec<i16>,
}

fn columns<'a>(declarations: impl Iterator<Item = &'a AuditDetailDeclaration>) -> Columns {
    let mut columns = Columns {
        classes: Vec::new(),
        schemas: Vec::new(),
        ocsf_classes: Vec::new(),
        ocsf_activities: Vec::new(),
    };
    for declaration in declarations {
        columns.classes.push(declaration.class().clone());
        columns.schemas.push(declaration.schema().cloned());
        columns
            .ocsf_classes
            .push(i32::try_from(declaration.ocsf().class_uid).unwrap_or(i32::MAX));
        columns
            .ocsf_activities
            .push(i16::from(declaration.ocsf().activity_id));
    }
    columns
}
