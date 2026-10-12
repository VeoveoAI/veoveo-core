//! Ed25519 verification keys as RFC 8037 JSON Web Keys. A server's key is identified by
//! its RFC 7638 thumbprint; the Gateway's access-token keys carry rotation key IDs.
use std::{collections::BTreeSet, fmt};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::DeclarationError;

/// An Ed25519 public key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServerPublicKey([u8; 32]);

impl ServerPublicKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
    /// The JWK `x` member: the key in base64url without padding.
    pub fn x(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }
    /// The RFC 7638 thumbprint: base64url SHA-256 over the required members in
    /// lexicographic order.
    pub fn thumbprint(&self) -> JwkKeyId {
        let canonical = format!(r#"{{"crv":"Ed25519","kty":"OKP","x":"{}"}}"#, self.x());
        JwkKeyId(URL_SAFE_NO_PAD.encode(Sha256::digest(canonical.as_bytes())))
    }
}

/// A JWK `kid`: 1..=128 ASCII letters, digits, dot, underscore or hyphen.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
pub struct JwkKeyId(String);

impl JwkKeyId {
    pub fn new(value: impl Into<String>) -> Result<Self, DeclarationError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(DeclarationError::invalid(
                "kid",
                "1..=128 ASCII letters, digits, dot, underscore or hyphen",
            ));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for JwkKeyId {
    type Error = DeclarationError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<JwkKeyId> for String {
    fn from(value: JwkKeyId) -> Self {
        value.0
    }
}
impl fmt::Display for JwkKeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, veoveo_types::Vocabulary)]
enum KeyType {
    #[vocabulary(rename = "OKP")]
    Okp,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, veoveo_types::Vocabulary)]
enum Curve {
    #[vocabulary(rename = "Ed25519")]
    Ed25519,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, veoveo_types::Vocabulary)]
enum SigningAlgorithm {
    #[vocabulary(rename = "EdDSA")]
    EdDsa,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, veoveo_types::Vocabulary)]
enum KeyUse {
    #[vocabulary(rename = "sig")]
    Signature,
}

/// One Ed25519 signature verification key. Every member is required, and private or
/// unknown members are refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "JwkWire", into = "JwkWire")]
#[schemars(with = "JwkWire")]
pub struct Ed25519Jwk {
    pub kid: JwkKeyId,
    pub key: ServerPublicKey,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "Ed25519Jwk")]
struct JwkWire {
    kty: KeyType,
    crv: Curve,
    /// The 32-byte public key in base64url without padding.
    #[schemars(length(min = 43, max = 43))]
    x: String,
    kid: JwkKeyId,
    alg: SigningAlgorithm,
    #[serde(rename = "use")]
    key_use: KeyUse,
}

impl TryFrom<JwkWire> for Ed25519Jwk {
    type Error = DeclarationError;
    fn try_from(wire: JwkWire) -> Result<Self, Self::Error> {
        let key = URL_SAFE_NO_PAD
            .decode(&wire.x)
            .ok()
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
            .ok_or(DeclarationError::invalid(
                "jwk",
                "x must be 32 bytes in base64url without padding",
            ))?;
        Ok(Self {
            kid: wire.kid,
            key: ServerPublicKey(key),
        })
    }
}
impl From<Ed25519Jwk> for JwkWire {
    fn from(jwk: Ed25519Jwk) -> Self {
        Self {
            kty: KeyType::Okp,
            crv: Curve::Ed25519,
            x: jwk.key.x(),
            kid: jwk.kid,
            alg: SigningAlgorithm::EdDsa,
            key_use: KeyUse::Signature,
        }
    }
}

/// A JWK Set of Ed25519 verification keys: at least one key, with distinct key IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "JwkSetWire", into = "JwkSetWire")]
#[schemars(with = "JwkSetWire")]
pub struct Ed25519JwkSet(Vec<Ed25519Jwk>);

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "Ed25519JwkSet")]
struct JwkSetWire {
    #[schemars(length(min = 1))]
    keys: Vec<Ed25519Jwk>,
}

impl Ed25519JwkSet {
    pub fn new(keys: Vec<Ed25519Jwk>) -> Result<Self, DeclarationError> {
        if keys.is_empty() {
            return Err(DeclarationError::invalid("jwks", "must contain a key"));
        }
        let mut kids = BTreeSet::new();
        if !keys.iter().all(|jwk| kids.insert(&jwk.kid)) {
            return Err(DeclarationError::invalid(
                "jwks",
                "key IDs must be distinct",
            ));
        }
        Ok(Self(keys))
    }
    pub fn keys(&self) -> &[Ed25519Jwk] {
        &self.0
    }
    pub fn get(&self, kid: &JwkKeyId) -> Option<&ServerPublicKey> {
        self.0
            .iter()
            .find(|jwk| &jwk.kid == kid)
            .map(|jwk| &jwk.key)
    }
}
impl TryFrom<JwkSetWire> for Ed25519JwkSet {
    type Error = DeclarationError;
    fn try_from(wire: JwkSetWire) -> Result<Self, Self::Error> {
        Self::new(wire.keys)
    }
}
impl From<Ed25519JwkSet> for JwkSetWire {
    fn from(set: Ed25519JwkSet) -> Self {
        Self { keys: set.0 }
    }
}

/// The `jwks` client metadata of a registering server: exactly one key, identified by
/// its RFC 7638 thumbprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "Ed25519JwkSet", into = "Ed25519JwkSet")]
#[schemars(with = "Ed25519JwkSet")]
pub struct ServerKeySet(ServerPublicKey);

impl ServerKeySet {
    pub fn new(key: ServerPublicKey) -> Self {
        Self(key)
    }
    pub fn key(&self) -> &ServerPublicKey {
        &self.0
    }
}
impl TryFrom<Ed25519JwkSet> for ServerKeySet {
    type Error = DeclarationError;
    fn try_from(set: Ed25519JwkSet) -> Result<Self, Self::Error> {
        match set.keys() {
            [jwk] if jwk.kid == jwk.key.thumbprint() => Ok(Self(jwk.key)),
            [_] => Err(DeclarationError::invalid(
                "jwks",
                "the key ID must be the key's RFC 7638 thumbprint",
            )),
            _ => Err(DeclarationError::invalid(
                "jwks",
                "must contain exactly one key",
            )),
        }
    }
}
impl From<ServerKeySet> for Ed25519JwkSet {
    fn from(set: ServerKeySet) -> Self {
        Self(vec![Ed25519Jwk {
            kid: set.0.thumbprint(),
            key: set.0,
        }])
    }
}
