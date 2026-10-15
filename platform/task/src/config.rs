//! Connection settings for a Task runtime process.
use secrecy::SecretString;
use veoveo_store::{InstanceGrace, StoreConfig, StoreError};

use crate::TaskError;

/// The installation database a runtime connects to, as a registration grant names it.
/// Debug output never shows the password.
#[derive(Clone, Debug)]
pub struct TaskRuntimeConfig {
    store: StoreConfig,
}

impl TaskRuntimeConfig {
    /// `url` is a `postgres://` URL without a password; `login` replaces a user the URL
    /// names.
    pub fn new(
        url: impl Into<String>,
        login: impl Into<String>,
        password: SecretString,
    ) -> Result<Self, TaskError> {
        let store = StoreConfig::builder(url, password)
            .username(login)
            .build()
            .map_err(StoreError::from)?;
        Ok(Self { store })
    }

    /// How long this process's reaper lets an instance's lock stay free before it moves
    /// the instance's Tasks to recovery, [`INSTANCE_GRACE`](crate::INSTANCE_GRACE) by
    /// default. Every process of an installation should use the same grace.
    pub fn instance_grace(self, grace: InstanceGrace) -> Self {
        Self {
            store: self.store.with_instance_grace(grace),
        }
    }

    pub fn store_config(&self) -> &StoreConfig {
        &self.store
    }
}

impl From<StoreConfig> for TaskRuntimeConfig {
    fn from(store: StoreConfig) -> Self {
        Self { store }
    }
}
