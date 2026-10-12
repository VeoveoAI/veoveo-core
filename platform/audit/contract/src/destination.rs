//! Export identities distinguish destination configuration from content hashes.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use veoveo_types::Sha256Digest;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct AuditDestinationId(Sha256Digest);
impl AuditDestinationId {
    pub fn from_configuration_hash(hash: Sha256Digest) -> Self {
        Self(hash)
    }
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Both objects are immutable. A retry must reproduce both byte streams.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditExportPayload {
    pub content: Sha256Digest,
    pub seal: Sha256Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, veoveo_types::Vocabulary)]
#[cfg_attr(feature = "postgres", vocabulary(postgres))]
pub enum AuditExportRejection {
    PartialAcceptance,
    Protocol,
    ContentConflict,
    ObjectLock,
    PayloadTooLarge,
}
