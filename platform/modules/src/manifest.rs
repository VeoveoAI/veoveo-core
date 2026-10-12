//! Wire form of an optional module declaration, in the snake_case of the OAuth
//! registration document that carries it. A manifest a server sends at
//! registration takes the same admission as the Rust declaration that produced it:
//! [`ModuleManifest::admit`] rebuilds every name, migration, lane and claim through
//! the checked constructors and [`ModuleSetup::builder`].
use crate::*;
use sha2::{Digest, Sha256};

/// One optional module, as a registering server declares it. Kernel modules are
/// compiled into the Gateway and never travel as manifests.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ModuleManifest {
    pub name: ModuleName,
    pub ownership: Vec<ManifestClaim>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<ManifestRequirement>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<ManifestExtension>,
    pub migrations: Vec<ManifestMigration>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", tag = "kind", content = "name")]
pub enum ManifestClaim {
    Table(TableName),
    TablePrefix(TablePrefix),
    Function(FunctionName),
    FunctionPrefix(FunctionPrefix),
    Analyzer(AnalyzerName),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ManifestRequirement {
    pub module: ModuleName,
    /// Absent: the dependency's whole lane must be current.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_least: Option<MigrationVersion>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ManifestExtension {
    pub point: ExtensionPointName,
    pub implementation: ExtensionImplementationName,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ManifestMigration {
    pub version: MigrationVersion,
    pub name: MigrationName,
    pub sql: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<ManifestRequirement>,
}

impl ModuleManifest {
    /// Describe a compiled optional module for registration.
    pub fn from_setup(setup: &ModuleSetup) -> Result<Self, DeclarationError> {
        if setup.layer() != ModuleLayer::Optional {
            return Err(DeclarationError::new(
                "only optional modules travel as manifests",
            ));
        }
        if !setup.sql_apis().is_empty() {
            return Err(DeclarationError::new(
                "optional modules export no kernel SQL API",
            ));
        }
        Ok(Self {
            name: setup.name().clone(),
            ownership: setup.ownership().iter().map(ManifestClaim::from).collect(),
            requires: setup
                .requires()
                .iter()
                .map(ManifestRequirement::from)
                .collect(),
            extensions: setup
                .extensions()
                .iter()
                .map(|binding| ManifestExtension {
                    point: binding.point().clone(),
                    implementation: binding.implementation().clone(),
                })
                .collect(),
            migrations: setup
                .lane()
                .migrations()
                .iter()
                .map(|migration| ManifestMigration {
                    version: migration.version(),
                    name: migration.name().clone(),
                    sql: migration.sql().to_owned(),
                    requires: migration
                        .requires()
                        .iter()
                        .map(ManifestRequirement::from)
                        .collect(),
                })
                .collect(),
        })
    }

    /// Admit a received manifest exactly as a compiled declaration. The receiver
    /// supplies the execution host, because a registering server never chooses where
    /// its migrations run.
    pub fn admit(&self, execution: LaneExecution) -> Result<ModuleSetup, DeclarationError> {
        let migrations = self
            .migrations
            .iter()
            .map(|migration| {
                Migration::new(
                    migration.version,
                    migration.name.clone(),
                    migration.sql.as_str(),
                )?
                .with_requirements(
                    migration
                        .requires
                        .iter()
                        .map(ManifestRequirement::admit)
                        .collect(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        ModuleSetup::builder(self.name.clone(), ModuleLayer::Optional)
            .ownership(self.ownership.iter().map(ManifestClaim::admit).collect())
            .lane(MigrationLane::new(migrations)?)
            .execution(execution)
            .requires(
                self.requires
                    .iter()
                    .map(ManifestRequirement::admit)
                    .collect(),
            )
            .extensions(
                self.extensions
                    .iter()
                    .map(|binding| {
                        ExtensionBinding::new(binding.point.clone(), binding.implementation.clone())
                    })
                    .collect(),
            )
            .build()
    }

    /// SHA-256 over the canonical JSON form, as lowercase hexadecimal. Equal
    /// declarations have equal digests.
    pub fn sha256(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("manifest serializes");
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// The latest migration version, absent for an empty lane.
    pub fn latest(&self) -> Option<MigrationVersion> {
        self.migrations.last().map(|migration| migration.version)
    }
}

impl From<&OwnershipClaim> for ManifestClaim {
    fn from(claim: &OwnershipClaim) -> Self {
        match claim {
            OwnershipClaim::Table(name) => Self::Table(name.clone()),
            OwnershipClaim::TablePrefix(name) => Self::TablePrefix(name.clone()),
            OwnershipClaim::Function(name) => Self::Function(name.clone()),
            OwnershipClaim::FunctionPrefix(name) => Self::FunctionPrefix(name.clone()),
            OwnershipClaim::Analyzer(name) => Self::Analyzer(name.clone()),
        }
    }
}

impl ManifestClaim {
    fn admit(&self) -> OwnershipClaim {
        match self {
            Self::Table(name) => OwnershipClaim::Table(name.clone()),
            Self::TablePrefix(name) => OwnershipClaim::TablePrefix(name.clone()),
            Self::Function(name) => OwnershipClaim::Function(name.clone()),
            Self::FunctionPrefix(name) => OwnershipClaim::FunctionPrefix(name.clone()),
            Self::Analyzer(name) => OwnershipClaim::Analyzer(name.clone()),
        }
    }
}

impl From<&LaneRequirement> for ManifestRequirement {
    fn from(requirement: &LaneRequirement) -> Self {
        Self {
            module: requirement.module().clone(),
            at_least: requirement.minimum(),
        }
    }
}

impl ManifestRequirement {
    fn admit(&self) -> LaneRequirement {
        match self.at_least {
            None => LaneRequirement::Satisfied(self.module.clone()),
            Some(version) => LaneRequirement::AtLeast {
                module: self.module.clone(),
                version,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn execution() -> LaneExecution {
        LaneExecution::new(
            ExecutionImage::new("gateway").unwrap(),
            ExecutionCommand::new(vec!["module-migrate".into()]).unwrap(),
        )
        .unwrap()
    }

    fn setup() -> ModuleSetup {
        ModuleSetup::builder(ModuleName::new("time").unwrap(), ModuleLayer::Optional)
            .ownership(vec![
                OwnershipClaim::TablePrefix(TablePrefix::new("time_").unwrap()),
                OwnershipClaim::FunctionPrefix(FunctionPrefix::new("fn::time::").unwrap()),
            ])
            .requires(vec![LaneRequirement::Satisfied(
                ModuleName::new("identity").unwrap(),
            )])
            .lane(
                MigrationLane::new(vec![
                    Migration::new(
                        MigrationVersion::new(0),
                        MigrationName::new("current").unwrap(),
                        "DEFINE TABLE time_event SCHEMAFULL;",
                    )
                    .unwrap(),
                    Migration::new(
                        MigrationVersion::new(1),
                        MigrationName::new("index").unwrap(),
                        "DEFINE INDEX time_event_due ON time_event FIELDS due;",
                    )
                    .unwrap(),
                ])
                .unwrap(),
            )
            .execution(execution())
            .build()
            .unwrap()
    }

    #[test]
    fn compiled_setup_round_trips_through_the_wire() {
        let manifest = ModuleManifest::from_setup(&setup()).unwrap();
        let wire = serde_json::to_value(&manifest).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({
                "name": "time",
                "ownership": [
                    {"kind": "table_prefix", "name": "time_"},
                    {"kind": "function_prefix", "name": "fn::time::"}
                ],
                "requires": [{"module": "identity"}],
                "migrations": [
                    {"version": 0, "name": "current", "sql": "DEFINE TABLE time_event SCHEMAFULL;"},
                    {"version": 1, "name": "index", "sql": "DEFINE INDEX time_event_due ON time_event FIELDS due;"}
                ]
            })
        );
        let received: ModuleManifest = serde_json::from_value(wire).unwrap();
        assert_eq!(received.admit(execution()).unwrap(), setup());
        assert_eq!(received.sha256(), manifest.sha256());
        assert_eq!(received.latest(), Some(MigrationVersion::new(1)));
    }

    #[test]
    fn received_manifests_take_declaration_admission() {
        let valid = serde_json::to_value(ModuleManifest::from_setup(&setup()).unwrap()).unwrap();
        let rejected = |mutate: fn(&mut serde_json::Value)| {
            let mut wire = valid.clone();
            mutate(&mut wire);
            match serde_json::from_value::<ModuleManifest>(wire) {
                Err(_) => true,
                Ok(manifest) => manifest.admit(execution()).is_err(),
            }
        };
        // Overlapping ownership claims.
        assert!(rejected(|w| {
            w["ownership"][1] = serde_json::json!({"kind": "table", "name": "time_event"})
        }));
        // Non-contiguous lane.
        assert!(rejected(
            |w| w["migrations"][1]["version"] = serde_json::json!(5)
        ));
        // Empty SQL body.
        assert!(rejected(
            |w| w["migrations"][0]["sql"] = serde_json::json!("  ")
        ));
        // Malformed names fail at the wire boundary.
        assert!(rejected(|w| w["name"] = serde_json::json!("Time")));
        assert!(rejected(|w| {
            w["ownership"][0] = serde_json::json!({"kind": "table_prefix", "name": "time-"})
        }));
        // A module cannot require itself.
        assert!(rejected(
            |w| w["requires"] = serde_json::json!([{"module": "time"}])
        ));
        // Unknown members are refused.
        assert!(rejected(|w| w["layer"] = serde_json::json!("kernel")));
        // A later migration cannot drop an earlier prerequisite.
        assert!(rejected(|w| {
            w["migrations"][0]["requires"] =
                serde_json::json!([{"module": "identity", "at_least": 2}])
        }));
    }

    #[test]
    fn kernel_modules_do_not_travel() {
        let kernel = ModuleSetup::builder(ModuleName::new("store").unwrap(), ModuleLayer::Kernel)
            .execution(execution())
            .build()
            .unwrap();
        assert!(ModuleManifest::from_setup(&kernel).is_err());
    }
}
