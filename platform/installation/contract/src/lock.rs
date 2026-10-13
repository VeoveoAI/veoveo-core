//! The component lock document, `https://veoveo.ai/formats/component-lock`.
use std::{borrow::Cow, collections::BTreeSet};

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize};
use veoveo_types::{Check, Checked};

use crate::{ComponentLockError, ComponentName, ComponentRepository, GitRevision};

/// The format URI of a component lock, its `format` member and its schema's `$id`.
pub const COMPONENT_LOCK_FORMAT: &str = "https://veoveo.ai/formats/component-lock";

/// The format of a component lock.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, veoveo_types::Vocabulary)]
#[schemars(transform = veoveo_types::naming::format_uri_naming)]
pub enum ComponentLockFormat {
    #[vocabulary(rename = "https://veoveo.ai/formats/component-lock")]
    ComponentLock,
}

/// One component pinned to a revision of its repository.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LockedComponent(Checked<LockedComponentValue>);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LockedComponentValue {
    name: ComponentName,
    repository: ComponentRepository,
    revision: GitRevision,
}

impl Check for LockedComponentValue {
    type Error = ComponentLockError;
    fn check(&self) -> Result<(), Self::Error> {
        if self.repository.name() != self.name {
            return Err(ComponentLockError::RepositoryName {
                name: self.name.clone(),
                repository: self.repository.clone(),
            });
        }
        Ok(())
    }
}

impl LockedComponent {
    /// A component pinned to `revision`. The repository's name is the component's name.
    pub fn new(
        name: ComponentName,
        repository: ComponentRepository,
        revision: GitRevision,
    ) -> Result<Self, ComponentLockError> {
        Checked::new(LockedComponentValue {
            name,
            repository,
            revision,
        })
        .map(Self)
    }

    pub fn name(&self) -> &ComponentName {
        &self.0.name
    }

    pub fn repository(&self) -> &ComponentRepository {
        &self.0.repository
    }

    pub fn revision(&self) -> &GitRevision {
        &self.0.revision
    }
}

impl JsonSchema for LockedComponent {
    fn schema_name() -> Cow<'static, str> {
        "LockedComponent".into()
    }

    fn schema_id() -> Cow<'static, str> {
        concat!(module_path!(), "::LockedComponent").into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mut schema = LockedComponentValue::json_schema(generator);
        schema.insert(
            "description".into(),
            "One component pinned to a revision of its repository; the repository's name is the component's name.".into(),
        );
        schema
    }
}

/// An installation's component lock: the repository and Git revision of each component it
/// runs, each component named once.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ComponentLock(Checked<ComponentLockValue>);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(rename = "ComponentLock")]
struct ComponentLockValue {
    format: ComponentLockFormat,
    #[schemars(length(min = 1))]
    components: Vec<LockedComponent>,
}

impl Check for ComponentLockValue {
    type Error = ComponentLockError;
    fn check(&self) -> Result<(), Self::Error> {
        if self.components.is_empty() {
            return Err(ComponentLockError::Empty);
        }
        let mut names = BTreeSet::new();
        for component in &self.components {
            if !names.insert(component.name()) {
                return Err(ComponentLockError::DuplicateComponent(
                    component.name().clone(),
                ));
            }
        }
        Ok(())
    }
}

impl ComponentLock {
    /// A lock of `components`, in their given order.
    pub fn new(components: Vec<LockedComponent>) -> Result<Self, ComponentLockError> {
        Checked::new(ComponentLockValue {
            format: ComponentLockFormat::ComponentLock,
            components,
        })
        .map(Self)
    }

    pub fn components(&self) -> &[LockedComponent] {
        &self.0.components
    }

    /// The pin of the component named `name`.
    pub fn component(&self, name: &ComponentName) -> Option<&LockedComponent> {
        self.components()
            .iter()
            .find(|component| component.name() == name)
    }
}

impl<'de> Deserialize<'de> for ComponentLock {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Checked::deserialize(deserializer).map(Self)
    }
}

impl JsonSchema for ComponentLock {
    fn schema_name() -> Cow<'static, str> {
        "ComponentLock".into()
    }

    fn schema_id() -> Cow<'static, str> {
        concat!(module_path!(), "::ComponentLock").into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mut schema = ComponentLockValue::json_schema(generator);
        schema.insert("$id".into(), COMPONENT_LOCK_FORMAT.into());
        schema.insert(
            "description".into(),
            "An installation's component lock: the repository and Git revision of each component it runs, each component named once.".into(),
        );
        schema
    }
}
