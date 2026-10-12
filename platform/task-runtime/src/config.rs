//! Connection settings for a Task runtime process.
use secrecy::SecretString;
use veoveo_platform_store::{StoreConfig, StoreError};

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

    pub fn store_config(&self) -> &StoreConfig {
        &self.store
    }
}

impl From<StoreConfig> for TaskRuntimeConfig {
    fn from(store: StoreConfig) -> Self {
        Self { store }
    }
}
