//! Server keys and the RFC 7591 software statement that proves a registering server
//! holds the key in its `jwks`.
//!
//! The statement is a JWT signed with the server's Ed25519 key. Its claims are the
//! request's client metadata together with the RFC 7523 §3 assertion claims: issuer and
//! subject name the server's slug, the audience is the URL the request is sent to, the
//! `jti` is a UUIDv7, and it lives at most 60 seconds. The Gateway rejects a statement
//! whose metadata differs from the request's, so the signature covers every declaration.
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use chrono::{DateTime, TimeDelta, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use veoveo_types::ServerSlug;

use crate::{ClientMetadata, RegistrationErrorCode, ServerPublicKey, SoftwareStatement};

/// The longest statement lifetime the Gateway admits.
pub const MAX_STATEMENT_LIFETIME: TimeDelta = TimeDelta::seconds(60);
/// Tolerated clock difference between server and Gateway.
pub const CLOCK_SKEW: TimeDelta = TimeDelta::seconds(5);

#[derive(Debug, thiserror::Error)]
pub enum StatementError {
    #[error("server key generation failed")]
    KeyGeneration,
    #[error("server key is not an Ed25519 PKCS#8 document")]
    InvalidKey,
    #[error("software statement signing failed")]
    Signing,
    #[error("the namespace is pinned to another key")]
    UnpinnedKey,
    #[error("software statement is not a JWT signed by the key in jwks")]
    InvalidSignature,
    #[error("software statement {0}")]
    Rejected(&'static str),
}

impl StatementError {
    /// The RFC 7591 error code a Gateway answers with.
    pub fn error_code(&self) -> RegistrationErrorCode {
        match self {
            Self::UnpinnedKey => RegistrationErrorCode::UnapprovedSoftwareStatement,
            _ => RegistrationErrorCode::InvalidSoftwareStatement,
        }
    }
}

/// A server's private key. The registration client's key directory persists it inside
/// the server's environment; the public half is pinned to the server's namespace.
pub struct ServerKeyPair {
    pkcs8: Vec<u8>,
    public: ServerPublicKey,
}

impl std::fmt::Debug for ServerKeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerKeyPair")
            .field("public", &self.public)
            .field("pkcs8", &"[REDACTED]")
            .finish()
    }
}

impl ServerKeyPair {
    pub fn generate() -> Result<Self, StatementError> {
        let document = Ed25519KeyPair::generate_pkcs8(&aws_lc_rs::rand::SystemRandom::new())
            .map_err(|_| StatementError::KeyGeneration)?;
        Self::from_pkcs8_der(document.as_ref())
    }

    pub fn from_pkcs8_der(der: &[u8]) -> Result<Self, StatementError> {
        let pair = Ed25519KeyPair::from_pkcs8(der).map_err(|_| StatementError::InvalidKey)?;
        let public = <[u8; 32]>::try_from(pair.public_key().as_ref())
            .map_err(|_| StatementError::InvalidKey)?;
        Ok(Self {
            pkcs8: der.to_vec(),
            public: ServerPublicKey::from_bytes(public),
        })
    }

    pub fn pkcs8_der(&self) -> &[u8] {
        &self.pkcs8
    }

    pub fn public_key(&self) -> &ServerPublicKey {
        &self.public
    }

    /// Sign the software statement for one request to `audience`.
    pub fn sign_statement(
        &self,
        metadata: &ClientMetadata,
        audience: &url::Url,
        now: DateTime<Utc>,
    ) -> Result<SoftwareStatement, StatementError> {
        if metadata.key() != &self.public {
            return Err(StatementError::Rejected(
                "metadata names another key than the signer",
            ));
        }
        ensure_crypto_provider();
        let claims = StatementClaims {
            iss: metadata.slug().clone(),
            sub: metadata.slug().clone(),
            aud: audience.as_str().to_owned(),
            iat: now.timestamp(),
            exp: (now + MAX_STATEMENT_LIFETIME).timestamp(),
            jti: uuid::Uuid::now_v7().to_string(),
            metadata: metadata.clone(),
        };
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = Some("JWT".into());
        header.kid = Some(self.public.thumbprint().into());
        let token = jsonwebtoken::encode(&header, &claims, &EncodingKey::from_ed_der(&self.pkcs8))
            .map_err(|_| StatementError::Signing)?;
        SoftwareStatement::new(token).map_err(|_| StatementError::Signing)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StatementClaims {
    iss: ServerSlug,
    sub: ServerSlug,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
    #[serde(flatten)]
    metadata: ClientMetadata,
}

/// Verify a software statement against the request's metadata and the URL the request
/// was sent to. `pinned` is the key the namespace is already pinned to, if any.
pub fn verify_statement(
    statement: &SoftwareStatement,
    metadata: &ClientMetadata,
    pinned: Option<&ServerPublicKey>,
    audience: &url::Url,
    now: DateTime<Utc>,
) -> Result<(), StatementError> {
    let key = metadata.key();
    if pinned.is_some_and(|pinned| pinned != key) {
        return Err(StatementError::UnpinnedKey);
    }
    ensure_crypto_provider();
    let header = jsonwebtoken::decode_header(statement.as_str())
        .map_err(|_| StatementError::InvalidSignature)?;
    let thumbprint = key.thumbprint();
    if header.alg != Algorithm::EdDSA || header.kid.as_deref() != Some(thumbprint.as_str()) {
        return Err(StatementError::Rejected("header must name the jwks key"));
    }
    let decoding =
        DecodingKey::from_ed_components(&key.x()).map_err(|_| StatementError::InvalidSignature)?;
    let slug = metadata.slug().as_str();
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.set_audience(&[audience.as_str()]);
    validation.set_issuer(&[slug]);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    validation.leeway = 0;
    validation.validate_exp = false;
    let claims =
        jsonwebtoken::decode::<StatementClaims>(statement.as_str(), &decoding, &validation)
            .map_err(|_| StatementError::InvalidSignature)?
            .claims;
    if claims.sub.as_str() != slug {
        return Err(StatementError::Rejected(
            "subject is not the registering server",
        ));
    }
    let issued = DateTime::from_timestamp(claims.iat, 0)
        .ok_or(StatementError::Rejected("has an invalid issue time"))?;
    let expires = DateTime::from_timestamp(claims.exp, 0)
        .ok_or(StatementError::Rejected("has an invalid expiry"))?;
    if issued > now + CLOCK_SKEW {
        return Err(StatementError::Rejected("was issued in the future"));
    }
    if expires <= now - CLOCK_SKEW {
        return Err(StatementError::Rejected("has expired"));
    }
    if expires - issued > MAX_STATEMENT_LIFETIME {
        return Err(StatementError::Rejected("lives longer than 60 seconds"));
    }
    if uuid::Uuid::parse_str(&claims.jti)
        .ok()
        .is_none_or(|jti| jti.get_version_num() != 7)
    {
        return Err(StatementError::Rejected("jti must be a UUIDv7"));
    }
    if &claims.metadata != metadata {
        return Err(StatementError::Rejected(
            "metadata differs from the request",
        ));
    }
    Ok(())
}

fn ensure_crypto_provider() {
    let _ = jsonwebtoken::crypto::aws_lc::DEFAULT_PROVIDER.install_default();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AuthorizationDeclaration, RegistrationDeclaration, Requirements, ServerEndpoint,
        ServerVersion,
    };
    use veoveo_types::ResourceScheme;

    fn metadata(key: &ServerKeyPair) -> ClientMetadata {
        ClientMetadata::new(
            *key.public_key(),
            RegistrationDeclaration {
                slug: ServerSlug::parse("time").unwrap(),
                uri_scheme: ResourceScheme::parse("time").unwrap(),
                version: ServerVersion::new("0.1.0").unwrap(),
                endpoint: ServerEndpoint::new(url::Url::parse("https://time.example/mcp").unwrap())
                    .unwrap(),
            },
            AuthorizationDeclaration {
                permissions: Vec::new(),
                requirements: Requirements::default(),
            },
        )
        .unwrap()
    }

    fn audience() -> url::Url {
        url::Url::parse("https://gateway.example/oauth/register").unwrap()
    }

    #[test]
    fn statements_verify_against_the_jwks_key_and_metadata() {
        let pair = ServerKeyPair::generate().unwrap();
        let metadata = metadata(&pair);
        let now = Utc::now();
        let statement = pair.sign_statement(&metadata, &audience(), now).unwrap();
        verify_statement(&statement, &metadata, None, &audience(), now).unwrap();
        verify_statement(
            &statement,
            &metadata,
            Some(pair.public_key()),
            &audience(),
            now,
        )
        .unwrap();
        let restored = ServerKeyPair::from_pkcs8_der(pair.pkcs8_der()).unwrap();
        assert_eq!(restored.public_key(), pair.public_key());
    }

    #[test]
    fn statements_bind_key_pin_metadata_audience_and_time() {
        let pair = ServerKeyPair::generate().unwrap();
        let metadata = metadata(&pair);
        let now = Utc::now();
        let statement = pair.sign_statement(&metadata, &audience(), now).unwrap();

        let other = ServerKeyPair::generate().unwrap();
        let pinned = verify_statement(
            &statement,
            &metadata,
            Some(other.public_key()),
            &audience(),
            now,
        )
        .unwrap_err();
        assert_eq!(
            pinned.error_code(),
            RegistrationErrorCode::UnapprovedSoftwareStatement
        );

        // A statement signed by another key than the one in jwks.
        let mut claimed = metadata.clone();
        claimed.jwks = crate::ServerKeySet::new(*other.public_key());
        assert!(verify_statement(&statement, &claimed, None, &audience(), now).is_err());
        assert!(other.sign_statement(&metadata, &audience(), now).is_err());

        let mut changed = metadata.clone();
        changed.registration.version = ServerVersion::new("0.2.0").unwrap();
        assert!(verify_statement(&statement, &changed, None, &audience(), now).is_err());

        let elsewhere = url::Url::parse("https://other.example/oauth/register").unwrap();
        assert!(verify_statement(&statement, &metadata, None, &elsewhere, now).is_err());

        let later = now + TimeDelta::seconds(120);
        assert!(verify_statement(&statement, &metadata, None, &audience(), later).is_err());
        let earlier = now - TimeDelta::seconds(30);
        assert!(verify_statement(&statement, &metadata, None, &audience(), earlier).is_err());
    }

    #[test]
    fn debug_output_redacts_the_private_key() {
        let pair = ServerKeyPair::generate().unwrap();
        assert!(format!("{pair:?}").contains("[REDACTED]"));
    }
}
