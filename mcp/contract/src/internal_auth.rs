use veoveo_gateway_contract::ProtectedResourceId;
mod request;
mod upload;
pub use request::GatewayRequestContext;
#[cfg(test)]
#[path = "internal_auth/request_tests.rs"]
mod request_tests;
pub use upload::*;

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, decode_header, encode,
    errors::Error as JwtError,
    jwk::{
        AlgorithmParameters, EllipticCurve, Jwk, JwkSet, KeyAlgorithm, KeyOperations, PublicKeyUse,
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{GatewayProfileId, JwtId, OAuthClientId, Principal, ServerSlug, TokenIssuer};
use veoveo_mcp_ext_registration::{
    AccessTokenTrust, Ed25519Jwk, Ed25519JwkSet, PermissionsDetail, ResourceIndicator,
    ServerPublicKey, issuer_url, statement::ServerKeyPair,
};
use veoveo_types::{
    GroupId, IdentifierError, InvocationAuthority, PermissionName, PrincipalId, RoleId,
};

/// The JOSE `typ` of every token the Gateway forwards: an RFC 9068 JWT access token.
pub const ACCESS_TOKEN_TYPE: &str = "at+jwt";
/// Public claim, under a collision-resistant URI name (RFC 7519 §4.2), carrying the
/// profile, actor, authority and request context.
pub const INVOCATION_CLAIM: &str = "https://veoveo.ai/claims/invocation";
/// Public claim of HTTP resource tokens.
pub const RESOURCE_INVOCATION_CLAIM: &str = "https://veoveo.ai/claims/resource-invocation";

/// Error text for a hosted-server request that arrived without the gateway's
/// internal identity, forwarded bearer, or authenticated HTTP context.
pub const GATEWAY_ROUTING_REQUIRED: &str =
    "This server only accepts requests routed through the Veoveo gateway.";

/// The Gateway's Ed25519 signing key. Its `kid` is the RFC 7638 thumbprint of its
/// public half.
#[derive(Clone, PartialEq, Eq)]
pub struct GatewayInternalSigningKey {
    key_id: String,
    public_key: ServerPublicKey,
    private_key_der: Vec<u8>,
}

impl GatewayInternalSigningKey {
    /// Admit an Ed25519 PKCS#8 private key.
    pub fn new(private_key_der: impl Into<Vec<u8>>) -> Result<Self, InternalTokenError> {
        ensure_jwt_crypto_provider();
        let private_key_der = private_key_der.into();
        if private_key_der.is_empty() {
            return Err(InternalTokenError::EmptyPrivateKey);
        }
        let public_key = *ServerKeyPair::from_pkcs8_der(&private_key_der)
            .map_err(|_| InternalTokenError::InvalidSigningKey)?
            .public_key();
        Ok(Self {
            key_id: public_key.thumbprint().as_str().to_owned(),
            public_key,
            private_key_der,
        })
    }

    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// The verification key servers receive in `veoveo_access_tokens`.
    pub fn verification_key(&self) -> Ed25519Jwk {
        Ed25519Jwk::new(self.public_key)
    }
}

impl fmt::Debug for GatewayInternalSigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GatewayInternalSigningKey")
            .field("key_id", &self.key_id)
            .field("private_key_der", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct GatewayInternalTrustBundle {
    keys: Arc<BTreeMap<String, Jwk>>,
}

impl GatewayInternalTrustBundle {
    pub fn new(jwks: JwkSet) -> Result<Self, InternalTokenError> {
        ensure_jwt_crypto_provider();
        if jwks.keys.is_empty() {
            return Err(InternalTokenError::EmptyTrustBundle);
        }
        let mut keys = BTreeMap::new();
        for jwk in jwks.keys {
            let thumbprint = validate_verification_jwk(&jwk)?;
            DecodingKey::from_jwk(&jwk).map_err(InternalTokenError::Jwt)?;
            let key_id = validate_key_id(
                jwk.common
                    .key_id
                    .clone()
                    .ok_or(InternalTokenError::MissingKeyId)?,
            )?;
            if key_id != thumbprint {
                return Err(InternalTokenError::KeyIdNotThumbprint(key_id));
            }
            if keys.insert(key_id.clone(), jwk).is_some() {
                return Err(InternalTokenError::DuplicateKeyId(key_id));
            }
        }
        Ok(Self {
            keys: Arc::new(keys),
        })
    }

    pub fn from_json(value: &str) -> Result<Self, InternalTokenError> {
        let jwks = serde_json::from_str(value).map_err(InternalTokenError::TrustBundleJson)?;
        Self::new(jwks)
    }

    /// The keys a registration response names in `veoveo_access_tokens`.
    pub fn from_key_set(keys: &Ed25519JwkSet) -> Result<Self, InternalTokenError> {
        let jwks = serde_json::to_value(keys)
            .and_then(serde_json::from_value)
            .map_err(InternalTokenError::TrustBundleJson)?;
        Self::new(jwks)
    }

    pub fn key_ids(&self) -> impl Iterator<Item = &str> {
        self.keys.keys().map(String::as_str)
    }
}

fn validate_key_id(value: String) -> Result<String, InternalTokenError> {
    if value.is_empty()
        || value.trim() != value
        || value.chars().any(|character| character.is_control())
    {
        return Err(InternalTokenError::InvalidKeyId);
    }
    Ok(value)
}

/// Admit an Ed25519 signature verification key and return its RFC 7638 thumbprint.
fn validate_verification_jwk(jwk: &Jwk) -> Result<String, InternalTokenError> {
    if jwk.common.key_algorithm != Some(KeyAlgorithm::EdDSA) {
        return Err(InternalTokenError::UnsupportedTrustKey);
    }
    if jwk
        .common
        .public_key_use
        .as_ref()
        .is_some_and(|usage| usage != &PublicKeyUse::Signature)
    {
        return Err(InternalTokenError::UnsupportedTrustKey);
    }
    if jwk
        .common
        .key_operations
        .as_ref()
        .is_some_and(|operations| {
            operations.is_empty()
                || operations
                    .iter()
                    .any(|operation| operation != &KeyOperations::Verify)
        })
    {
        return Err(InternalTokenError::UnsupportedTrustKey);
    }
    match &jwk.algorithm {
        AlgorithmParameters::OctetKeyPair(parameters)
            if parameters.curve == EllipticCurve::Ed25519 =>
        {
            URL_SAFE_NO_PAD
                .decode(&parameters.x)
                .ok()
                .and_then(|key| <[u8; 32]>::try_from(key).ok())
                .map(|key| {
                    ServerPublicKey::from_bytes(key)
                        .thumbprint()
                        .as_str()
                        .to_owned()
                })
                .ok_or(InternalTokenError::UnsupportedTrustKey)
        }
        _ => Err(InternalTokenError::UnsupportedTrustKey),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GatewayInternalIdentity {
    pub issuer: TokenIssuer,
    pub profile: GatewayProfileId,
    pub server: ServerSlug,
    /// The token's `aud`: the server's resource URL.
    #[schemars(with = "String")]
    pub audience: ResourceIndicator,
    /// The OAuth client the caller used to reach the Gateway.
    pub client: OAuthClientId,
    pub actor: Principal,
    pub authority: InvocationAuthority,
    /// The permissions the caller holds in this server's namespace, as the Gateway
    /// granted them for this request. The server checks them again.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub permissions: BTreeSet<PermissionName>,
    /// Required by consumers that admit renewable grants or recheck accepted work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_context: Option<GatewayRequestContext>,
    pub jwt_id: JwtId,
    pub issued_at: DateTime<Utc>,
    pub not_before: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl GatewayInternalIdentity {
    /// Whether the Gateway granted `permission` for this request.
    pub fn grants(&self, permission: &PermissionName) -> bool {
        self.permissions.contains(permission)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct IssuedGatewayInternalToken {
    pub bearer_token: String,
    pub identity: GatewayInternalIdentity,
}

impl fmt::Debug for IssuedGatewayInternalToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IssuedGatewayInternalToken")
            .field("bearer_token", &"<redacted>")
            .field("identity", &self.identity)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct GatewayInternalTokenIssuer {
    issuer: TokenIssuer,
    signing_key: GatewayInternalSigningKey,
}

impl GatewayInternalTokenIssuer {
    /// `issuer` is the Gateway's HTTPS issuer URL, the value its RFC 8414 metadata
    /// announces.
    pub fn new(
        issuer: TokenIssuer,
        signing_key: GatewayInternalSigningKey,
    ) -> Result<Self, InternalTokenError> {
        issuer_url(&issuer).map_err(|_| InternalTokenError::InvalidIssuer)?;
        Ok(Self {
            issuer,
            signing_key,
        })
    }

    /// Issue the access token for one forwarded request. Every granted permission
    /// must belong to the audience server's namespace, and a request context must name
    /// the same OAuth client.
    pub fn issue(
        &self,
        request: ForwardedRequest,
    ) -> Result<IssuedGatewayInternalToken, InternalTokenError> {
        if let Some(foreign) = request
            .permissions
            .iter()
            .find(|permission| !permission.is_owned_by(&request.server))
        {
            return Err(InternalTokenError::PermissionOutsideAudience(
                foreign.clone(),
            ));
        }
        let identity = self.create_identity(request)?;
        let bearer_token = self.sign(&AccessTokenClaims::from_identity(&identity))?;
        Ok(IssuedGatewayInternalToken {
            bearer_token,
            identity,
        })
    }

    /// Sign claims as an RFC 9068 access token.
    fn sign(&self, claims: &impl Serialize) -> Result<String, InternalTokenError> {
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = Some(ACCESS_TOKEN_TYPE.to_owned());
        header.kid = Some(self.signing_key.key_id.clone());
        encode(
            &header,
            claims,
            &EncodingKey::from_ed_der(&self.signing_key.private_key_der),
        )
        .map_err(InternalTokenError::Jwt)
    }

    fn create_identity(
        &self,
        request: ForwardedRequest,
    ) -> Result<GatewayInternalIdentity, InternalTokenError> {
        let ForwardedRequest {
            profile,
            server,
            audience,
            client,
            actor,
            authority,
            permissions,
            request_context,
            expires_at,
        } = request;
        ensure_jwt_crypto_provider();
        let now = Utc::now();
        let expires_at = if let Some(context) = &request_context {
            context.validate_for(&actor, &authority)?;
            if context.access_token.oauth_client_id != client {
                return Err(InternalTokenError::InvalidRequestContext);
            }
            expires_at.min(context.access_token.expires_at)
        } else {
            expires_at
        };
        if expires_at <= now {
            return Err(InternalTokenError::ExpiredDelegation);
        }
        Ok(GatewayInternalIdentity {
            issuer: self.issuer.clone(),
            profile,
            server,
            audience,
            client,
            actor,
            authority,
            permissions,
            request_context,
            jwt_id: fresh_jwt_id()?,
            issued_at: now,
            not_before: now,
            expires_at,
        })
    }

    pub fn issue_resource(
        &self,
        protected_resource: ProtectedResourceId,
        server: ServerSlug,
        client: OAuthClientId,
        actor: Principal,
        authority: InvocationAuthority,
        expires_at: DateTime<Utc>,
    ) -> Result<IssuedGatewayInternalResourceToken, InternalTokenError> {
        ensure_jwt_crypto_provider();
        let now = Utc::now();
        if expires_at <= now {
            return Err(InternalTokenError::ExpiredDelegation);
        }
        let jwt_id = fresh_jwt_id()?;
        let identity = GatewayInternalResourceIdentity {
            issuer: self.issuer.clone(),
            protected_resource,
            server,
            client,
            actor,
            authority,
            jwt_id,
            issued_at: now,
            not_before: now,
            expires_at,
        };
        let bearer_token = self.sign(&ResourceAccessTokenClaims::from_identity(&identity))?;
        Ok(IssuedGatewayInternalResourceToken {
            bearer_token,
            identity,
        })
    }
}

/// What the Gateway forwards for one admitted request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardedRequest {
    pub profile: GatewayProfileId,
    /// The registered server that receives the request.
    pub server: ServerSlug,
    /// The token's `aud`: that server's resource URL, its registered MCP endpoint.
    pub audience: ResourceIndicator,
    /// The OAuth client the caller used to reach the Gateway.
    pub client: OAuthClientId,
    pub actor: Principal,
    pub authority: InvocationAuthority,
    /// The permissions the Gateway granted in `server`'s namespace.
    pub permissions: BTreeSet<PermissionName>,
    pub request_context: Option<GatewayRequestContext>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GatewayInternalResourceIdentity {
    pub issuer: TokenIssuer,
    pub protected_resource: ProtectedResourceId,
    pub server: ServerSlug,
    /// The OAuth client the caller used to reach the Gateway.
    pub client: OAuthClientId,
    pub actor: Principal,
    pub authority: InvocationAuthority,
    pub jwt_id: JwtId,
    pub issued_at: DateTime<Utc>,
    pub not_before: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct IssuedGatewayInternalResourceToken {
    pub bearer_token: String,
    pub identity: GatewayInternalResourceIdentity,
}

impl fmt::Debug for IssuedGatewayInternalResourceToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IssuedGatewayInternalResourceToken")
            .field("bearer_token", &"<redacted>")
            .field("identity", &self.identity)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct GatewayInternalResourceTokenVerifier {
    issuer: TokenIssuer,
    protected_resource: ProtectedResourceId,
    server: ServerSlug,
    trust_bundle: GatewayInternalTrustBundle,
}

impl GatewayInternalResourceTokenVerifier {
    /// Verify tokens whose `aud` is `protected_resource`, addressed to `server`.
    pub fn new(
        issuer: TokenIssuer,
        protected_resource: ProtectedResourceId,
        server: ServerSlug,
        trust_bundle: GatewayInternalTrustBundle,
    ) -> Self {
        Self {
            issuer,
            protected_resource,
            server,
            trust_bundle,
        }
    }

    pub fn verify(
        &self,
        bearer_token: &str,
    ) -> Result<GatewayInternalResourceIdentity, InternalTokenError> {
        let claims: ResourceAccessTokenClaims = decode_access_token(
            bearer_token,
            &self.issuer,
            &[self.protected_resource.as_str()],
            &self.trust_bundle,
        )?;
        let invocation = claims.invocation;
        if invocation.server != self.server
            || invocation.protected_resource.as_str() != claims.standard.aud
        {
            return Err(InternalTokenError::AudienceMismatch {
                expected: self.server.clone(),
                actual: invocation.server,
            });
        }
        claims.standard.check_actor(&invocation.actor)?;
        Ok(GatewayInternalResourceIdentity {
            issuer: claims.standard.iss,
            protected_resource: invocation.protected_resource,
            server: invocation.server,
            client: claims.standard.client_id,
            actor: invocation.actor,
            authority: invocation.authority,
            jwt_id: claims.standard.jti,
            issued_at: timestamp_to_datetime(claims.standard.iat, "iat")?,
            not_before: timestamp_to_datetime(claims.standard.nbf, "nbf")?,
            expires_at: timestamp_to_datetime(claims.standard.exp, "exp")?,
        })
    }
}

/// One server a verifier admits tokens for, and the resource URL its tokens name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenAudience {
    pub server: ServerSlug,
    pub resource: ResourceIndicator,
}

#[derive(Debug, Clone)]
pub struct GatewayInternalTokenVerifier {
    issuer: TokenIssuer,
    audiences: Vec<TokenAudience>,
    trust_bundle: GatewayInternalTrustBundle,
}

impl GatewayInternalTokenVerifier {
    /// Verify tokens addressed to exactly one server (the common upstream case).
    pub fn new(
        issuer: TokenIssuer,
        audience: TokenAudience,
        trust_bundle: GatewayInternalTrustBundle,
    ) -> Self {
        Self {
            issuer,
            audiences: vec![audience],
            trust_bundle,
        }
    }

    /// Verify the tokens a registration response's `veoveo_access_tokens` describes,
    /// addressed to `server`.
    pub fn from_access_tokens(
        trust: &AccessTokenTrust,
        server: ServerSlug,
    ) -> Result<Self, InternalTokenError> {
        Ok(Self::new(
            trust.issuer.clone(),
            TokenAudience {
                server,
                resource: trust.resource.clone(),
            },
            GatewayInternalTrustBundle::from_key_set(&trust.jwks)?,
        ))
    }

    /// Verify tokens addressed to any of `audiences`.
    ///
    /// The shared artifact plane uses this: a domain server forwards the
    /// gateway token it received (addressed to *that* server) on the principal's
    /// behalf, so the plane must accept the set of known upstream servers rather
    /// than a single audience. The gateway remains the only minter, so trust is
    /// unchanged; only which forwarded audiences are honored widens.
    pub fn new_for_audiences(
        issuer: TokenIssuer,
        audiences: Vec<TokenAudience>,
        trust_bundle: GatewayInternalTrustBundle,
    ) -> Self {
        Self {
            issuer,
            audiences,
            trust_bundle,
        }
    }

    pub fn verify(
        &self,
        bearer_token: &str,
    ) -> Result<GatewayInternalIdentity, InternalTokenError> {
        let claims = self.decode_claims::<AccessTokenClaims>(bearer_token)?;
        self.identity_from_claims(claims)
    }

    fn decode_claims<T: serde::de::DeserializeOwned>(
        &self,
        bearer_token: &str,
    ) -> Result<T, InternalTokenError> {
        let audiences: Vec<&str> = self
            .audiences
            .iter()
            .map(|audience| audience.resource.as_str())
            .collect();
        decode_access_token(bearer_token, &self.issuer, &audiences, &self.trust_bundle)
    }

    fn identity_from_claims(
        &self,
        claims: AccessTokenClaims,
    ) -> Result<GatewayInternalIdentity, InternalTokenError> {
        let server = claims.invocation.server.clone();
        let Some(audience) = self.audiences.iter().find(|audience| {
            audience.server == server && audience.resource.as_str() == claims.standard.aud
        }) else {
            return Err(InternalTokenError::AudienceMismatch {
                expected: self
                    .audiences
                    .first()
                    .map_or_else(|| server.clone(), |audience| audience.server.clone()),
                actual: server,
            });
        };
        let audience = audience.resource.clone();
        let invocation = claims.invocation;
        claims.standard.check_actor(&invocation.actor)?;
        let permissions = match claims.authorization_details.as_slice() {
            [] => BTreeSet::new(),
            [detail] => {
                if let Some(foreign) = detail
                    .actions
                    .iter()
                    .find(|permission| !permission.is_owned_by(&server))
                {
                    return Err(InternalTokenError::PermissionOutsideAudience(
                        foreign.clone(),
                    ));
                }
                detail.actions.clone()
            }
            _ => return Err(InternalTokenError::InconsistentClaims),
        };
        if let Some(context) = &invocation.request_context {
            context.validate_for(&invocation.actor, &invocation.authority)?;
            if claims.standard.exp > context.access_token.expires_at.timestamp()
                || context.access_token.oauth_client_id != claims.standard.client_id
            {
                return Err(InternalTokenError::InvalidRequestContext);
            }
        }
        Ok(GatewayInternalIdentity {
            issuer: claims.standard.iss,
            profile: invocation.profile,
            server,
            audience,
            client: claims.standard.client_id,
            actor: invocation.actor,
            authority: invocation.authority,
            permissions,
            request_context: invocation.request_context,
            jwt_id: claims.standard.jti,
            issued_at: timestamp_to_datetime(claims.standard.iat, "iat")?,
            not_before: timestamp_to_datetime(claims.standard.nbf, "nbf")?,
            expires_at: timestamp_to_datetime(claims.standard.exp, "exp")?,
        })
    }
}

/// Verify an RFC 9068 access token's header, signature and registered claims.
fn decode_access_token<T: serde::de::DeserializeOwned>(
    bearer_token: &str,
    issuer: &TokenIssuer,
    audiences: &[&str],
    trust_bundle: &GatewayInternalTrustBundle,
) -> Result<T, InternalTokenError> {
    ensure_jwt_crypto_provider();
    let header = decode_header(bearer_token).map_err(InternalTokenError::Jwt)?;
    if header.alg != Algorithm::EdDSA {
        return Err(InternalTokenError::UnsupportedTokenAlgorithm(header.alg));
    }
    // RFC 9068 §4: reject any type other than `at+jwt` or `application/at+jwt`.
    if !header.typ.as_deref().is_some_and(|typ| {
        typ.eq_ignore_ascii_case(ACCESS_TOKEN_TYPE)
            || typ.eq_ignore_ascii_case("application/at+jwt")
    }) {
        return Err(InternalTokenError::UnsupportedTokenType);
    }
    let key_id = header.kid.ok_or(InternalTokenError::MissingKeyId)?;
    let jwk = trust_bundle
        .keys
        .get(&key_id)
        .ok_or_else(|| InternalTokenError::UnknownKeyId(key_id.clone()))?;
    let decoding_key = DecodingKey::from_jwk(jwk).map_err(InternalTokenError::Jwt)?;
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.algorithms = vec![Algorithm::EdDSA];
    validation.validate_nbf = true;
    validation.leeway = 0;
    validation.set_issuer(&[issuer.as_str()]);
    validation.set_audience(audiences);
    validation.set_required_spec_claims(&["exp", "iat", "iss", "aud", "sub", "nbf"]);
    Ok(decode::<T>(bearer_token, &decoding_key, &validation)
        .map_err(InternalTokenError::Jwt)?
        .claims)
}

/// A fresh `jti`: a UUIDv7.
fn fresh_jwt_id() -> Result<JwtId, InternalTokenError> {
    JwtId::parse(uuid::Uuid::now_v7().to_string()).map_err(InternalTokenError::Identifier)
}

fn ensure_jwt_crypto_provider() {
    let _ = jsonwebtoken::crypto::aws_lc::DEFAULT_PROVIDER.install_default();
}

/// RFC 9068 §2.2 claims of every token the Gateway forwards. `iss` is the Gateway's
/// issuer URL, `client_id` the OAuth client the caller used, and `act` (RFC 8693 §4.1)
/// names the Gateway as the party acting for the caller. `sub`, `groups`, `roles` and
/// `scope` restate the embedded actor for standard resource-server libraries; a
/// verifier rejects a token in which they disagree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StandardClaims {
    iss: TokenIssuer,
    sub: String,
    aud: String,
    client_id: OAuthClientId,
    exp: i64,
    nbf: i64,
    iat: i64,
    jti: JwtId,
    act: ActingParty,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    groups: BTreeSet<GroupId>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    roles: BTreeSet<RoleId>,
}

/// RFC 8693 `act`: the Gateway, identified by its issuer URL. A nested actor chain is
/// refused until delegated tokens define one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActingParty {
    sub: TokenIssuer,
}

impl StandardClaims {
    #[allow(clippy::too_many_arguments)]
    fn new(
        issuer: &TokenIssuer,
        client: &OAuthClientId,
        actor: &Principal,
        audience: &str,
        jwt_id: &JwtId,
        issued_at: DateTime<Utc>,
        not_before: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            iss: issuer.clone(),
            sub: actor.id.as_str().to_owned(),
            aud: audience.to_owned(),
            client_id: client.clone(),
            exp: expires_at.timestamp(),
            nbf: not_before.timestamp(),
            iat: issued_at.timestamp(),
            jti: jwt_id.clone(),
            act: ActingParty {
                sub: issuer.clone(),
            },
            scope: scope_claim(actor),
            groups: actor.groups.clone(),
            roles: actor.roles.clone(),
        }
    }

    fn check_actor(&self, actor: &Principal) -> Result<(), InternalTokenError> {
        if uuid::Uuid::parse_str(self.jti.as_str())
            .ok()
            .is_none_or(|jti| jti.get_version_num() != 7)
        {
            return Err(InternalTokenError::InvalidTokenId);
        }
        if PrincipalId::parse(self.sub.clone()).map_err(InternalTokenError::Identifier)? != actor.id
        {
            return Err(InternalTokenError::SubjectPrincipalMismatch);
        }
        if self.act.sub != self.iss
            || self.groups != actor.groups
            || self.roles != actor.roles
            || self.scope != scope_claim(actor)
        {
            return Err(InternalTokenError::InconsistentClaims);
        }
        Ok(())
    }
}

/// RFC 9068 `scope`: the actor's scopes, space-delimited, or absent when it has none.
fn scope_claim(actor: &Principal) -> Option<String> {
    (!actor.scopes.is_empty()).then(|| {
        actor
            .scopes
            .iter()
            .map(|scope| scope.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AccessTokenClaims {
    #[serde(flatten)]
    standard: StandardClaims,
    /// RFC 9396: at most one [`PermissionsDetail`], absent when nothing is granted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    authorization_details: Vec<PermissionsDetail>,
    #[serde(rename = "https://veoveo.ai/claims/invocation")]
    invocation: Invocation,
}

/// The [`INVOCATION_CLAIM`]: who acts, under which authority and request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Invocation {
    profile: GatewayProfileId,
    server: ServerSlug,
    actor: Principal,
    authority: InvocationAuthority,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_context: Option<GatewayRequestContext>,
}

impl AccessTokenClaims {
    fn from_identity(identity: &GatewayInternalIdentity) -> Self {
        Self {
            standard: StandardClaims::new(
                &identity.issuer,
                &identity.client,
                &identity.actor,
                identity.audience.as_str(),
                &identity.jwt_id,
                identity.issued_at,
                identity.not_before,
                identity.expires_at,
            ),
            authorization_details: (!identity.permissions.is_empty())
                .then(|| PermissionsDetail::new(identity.permissions.clone()))
                .into_iter()
                .collect(),
            invocation: Invocation {
                profile: identity.profile.clone(),
                server: identity.server.clone(),
                actor: identity.actor.clone(),
                authority: identity.authority.clone(),
                request_context: identity.request_context.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ResourceAccessTokenClaims {
    #[serde(flatten)]
    standard: StandardClaims,
    #[serde(rename = "https://veoveo.ai/claims/resource-invocation")]
    invocation: ResourceInvocation,
}

/// The [`RESOURCE_INVOCATION_CLAIM`] of an HTTP resource token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceInvocation {
    protected_resource: ProtectedResourceId,
    server: ServerSlug,
    actor: Principal,
    authority: InvocationAuthority,
}

impl ResourceAccessTokenClaims {
    fn from_identity(identity: &GatewayInternalResourceIdentity) -> Self {
        Self {
            standard: StandardClaims::new(
                &identity.issuer,
                &identity.client,
                &identity.actor,
                identity.protected_resource.as_str(),
                &identity.jwt_id,
                identity.issued_at,
                identity.not_before,
                identity.expires_at,
            ),
            invocation: ResourceInvocation {
                protected_resource: identity.protected_resource.clone(),
                server: identity.server.clone(),
                actor: identity.actor.clone(),
                authority: identity.authority.clone(),
            },
        }
    }
}

#[derive(Debug)]
pub enum InternalTokenError {
    EmptyPrivateKey,
    /// The signing key is not an Ed25519 PKCS#8 document.
    InvalidSigningKey,
    /// The issuer is not an HTTPS URL without credentials, query or fragment.
    InvalidIssuer,
    /// A trust key's `kid` is not its RFC 7638 thumbprint.
    KeyIdNotThumbprint(String),
    /// The token's `jti` is not a UUIDv7.
    InvalidTokenId,
    InvalidKeyId,
    EmptyTrustBundle,
    MissingKeyId,
    DuplicateKeyId(String),
    UnknownKeyId(String),
    UnsupportedTrustKey,
    UnsupportedTokenAlgorithm(Algorithm),
    /// The JOSE header `typ` is not `at+jwt` (RFC 9068 §4).
    UnsupportedTokenType,
    ExpiredDelegation,
    AudienceMismatch {
        expected: ServerSlug,
        actual: ServerSlug,
    },
    SubjectPrincipalMismatch,
    /// Standard claims disagree with the embedded actor, `act` does not name the
    /// issuer, or the token carries more than one permissions detail.
    InconsistentClaims,
    /// A granted permission names another server's namespace.
    PermissionOutsideAudience(PermissionName),
    InvalidRequestContext,
    InvalidTimestamp {
        claim: &'static str,
        value: i64,
    },
    Identifier(IdentifierError),
    TrustBundleJson(serde_json::Error),
    Jwt(JwtError),
}

impl fmt::Display for InternalTokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPrivateKey => f.write_str("internal signing private key is empty"),
            Self::InvalidSigningKey => {
                f.write_str("internal signing key is not an Ed25519 PKCS#8 document")
            }
            Self::InvalidIssuer => f.write_str("token issuer must be an https URL"),
            Self::KeyIdNotThumbprint(key_id) => {
                write!(
                    f,
                    "trust key `{key_id}` is not named by its RFC 7638 thumbprint"
                )
            }
            Self::InvalidTokenId => f.write_str("token jti must be a UUIDv7"),
            Self::InvalidKeyId => f.write_str("internal signing key id is empty or invalid"),
            Self::EmptyTrustBundle => f.write_str("internal trust JWKS contains no keys"),
            Self::MissingKeyId => f.write_str("internal token or trust key is missing kid"),
            Self::DuplicateKeyId(key_id) => {
                write!(f, "internal trust JWKS contains duplicate kid `{key_id}`")
            }
            Self::UnknownKeyId(key_id) => {
                write!(f, "internal token references unknown kid `{key_id}`")
            }
            Self::UnsupportedTrustKey => {
                f.write_str("internal trust JWKS must contain Ed25519 verification keys")
            }
            Self::UnsupportedTokenAlgorithm(algorithm) => {
                write!(f, "internal token algorithm `{algorithm:?}` is not EdDSA")
            }
            Self::UnsupportedTokenType => {
                f.write_str("forwarded token type must be at+jwt (RFC 9068)")
            }
            Self::ExpiredDelegation => {
                f.write_str("internal token expiration is not in the future")
            }
            Self::AudienceMismatch { expected, actual } => write!(
                f,
                "internal token audience mismatch: expected `{expected}`, got `{actual}`"
            ),
            Self::PermissionOutsideAudience(permission) => write!(
                f,
                "internal token grants `{permission}` outside its audience server"
            ),
            Self::SubjectPrincipalMismatch => {
                f.write_str("internal token subject does not match embedded principal")
            }
            Self::InconsistentClaims => {
                f.write_str("internal token standard claims disagree with its invocation or grants")
            }
            Self::InvalidRequestContext => {
                f.write_str("internal token request context does not match its actor and authority")
            }
            Self::InvalidTimestamp { claim, value } => {
                write!(
                    f,
                    "internal token claim `{claim}` has invalid timestamp `{value}`"
                )
            }
            Self::Identifier(err) => write!(f, "invalid internal token identifier: {err}"),
            Self::TrustBundleJson(err) => write!(f, "invalid internal trust JWKS: {err}"),
            Self::Jwt(err) => write!(f, "internal token JWT validation failed: {err}"),
        }
    }
}

impl std::error::Error for InternalTokenError {}

fn timestamp_to_datetime(
    value: i64,
    claim: &'static str,
) -> Result<DateTime<Utc>, InternalTokenError> {
    DateTime::from_timestamp(value, 0).ok_or(InternalTokenError::InvalidTimestamp { claim, value })
}

#[cfg(test)]
pub(crate) mod tests {
    use chrono::TimeDelta;

    use super::*;

    pub(crate) use crate::hosting::testing::{
        TEST_CLIENT, TEST_ISSUER, authority, principal, signing_key, trust_bundle,
    };

    /// The resource URL of test server `server`.
    pub(crate) fn resource(server: &str) -> ResourceIndicator {
        ResourceIndicator::parse(&format!("https://{server}.veoveo.test/mcp")).unwrap()
    }

    /// The test Gateway's token issuer.
    pub(crate) fn issuer() -> GatewayInternalTokenIssuer {
        GatewayInternalTokenIssuer::new(TokenIssuer::parse(TEST_ISSUER).unwrap(), signing_key())
            .unwrap()
    }

    /// A verifier for tokens addressed to test server `server`.
    pub(crate) fn verifier(server: &str) -> GatewayInternalTokenVerifier {
        GatewayInternalTokenVerifier::new(
            TokenIssuer::parse(TEST_ISSUER).unwrap(),
            TokenAudience {
                server: ServerSlug::parse(server).unwrap(),
                resource: resource(server),
            },
            trust_bundle(),
        )
    }

    /// A request from the test user through the test client, granting `permissions`.
    pub(crate) fn forwarded(server: &str, permissions: &[&str]) -> ForwardedRequest {
        ForwardedRequest {
            profile: GatewayProfileId::parse("default").unwrap(),
            server: ServerSlug::parse(server).unwrap(),
            audience: resource(server),
            client: OAuthClientId::parse(TEST_CLIENT).unwrap(),
            actor: principal(),
            authority: authority(),
            permissions: permissions
                .iter()
                .map(|permission| PermissionName::parse(*permission).unwrap())
                .collect(),
            request_context: None,
            expires_at: Utc::now() + TimeDelta::minutes(5),
        }
    }

    #[test]
    fn upload_assertions_require_dedicated_audience_and_signed_policy_binding() {
        let issuer = issuer();
        let upload = veoveo_artifact_contract::ARTIFACT_UPLOAD_AUDIENCE;
        let verifier = verifier(upload);
        let binding = veoveo_artifact_contract::ArtifactUploadAuthority {
            control_plane_sha256: veoveo_artifact_contract::UploadSha256::parse("a".repeat(64))
                .unwrap(),
            context_digest: veoveo_artifact_contract::UploadSha256::parse("b".repeat(64)).unwrap(),
        };
        let actor = principal();
        let invocation = authority();
        let now = Utc::now();
        let request_context = GatewayRequestContext {
            audit: crate::audit::AuditRequest::background(),
            principal: actor.clone(),
            access_token: crate::AccessTokenSubject {
                managed_execution: None,
                issuer: actor.issuer.clone(),
                subject: actor.subject.clone(),
                oauth_client_id: crate::OAuthClientId::parse("upload-test").unwrap(),
                session_family: None,
                audience: veoveo_gateway_contract::ProtectedResourceId::parse(
                    "https://gateway.test/mcp/default",
                )
                .unwrap(),
                work_context: invocation.work_context.clone(),
                invocation_mode: invocation.provenance.mode(),
                initiator: invocation.provenance.initiator().cloned(),
                delegation_id: None,
                scopes: actor.scopes.clone(),
                jwt_id: None,
                issued_at: now,
                not_before: None,
                expires_at: now + TimeDelta::minutes(5),
            },
        };
        let token = issuer
            .issue_artifact_upload(
                GatewayProfileId::parse("default").unwrap(),
                resource(upload),
                actor,
                invocation,
                request_context,
                binding.clone(),
                Utc::now() + TimeDelta::minutes(1),
            )
            .unwrap();
        let verified = verifier.verify_artifact_upload(&token).unwrap();
        assert_eq!(verified.authorization, binding);
        assert_eq!(verified.identity.actor.id, principal().id);
        let ordinary = issuer.issue(forwarded(upload, &[])).unwrap();
        assert!(
            verifier
                .verify_artifact_upload(&ordinary.bearer_token)
                .is_err(),
            "a token without a signed policy binding was accepted"
        );
        assert!(self::verifier("media").verify(&token).is_err());
        let mut segments: Vec<String> = token.split('.').map(str::to_owned).collect();
        let mut claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(&segments[1]).unwrap()).unwrap();
        claims[ARTIFACT_UPLOAD_CLAIM]["context_digest"] = serde_json::json!("c".repeat(64));
        segments[1] = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
        assert!(
            verifier
                .verify_artifact_upload(&segments.join("."))
                .is_err()
        );
    }

    #[test]
    fn identities_carry_only_the_audience_servers_granted_permissions() {
        let issuer = issuer();
        let verifier = verifier("time");
        let read = PermissionName::parse("time.read").unwrap();
        let issued = issuer.issue(forwarded("time", &["time.read"])).unwrap();
        let identity = verifier.verify(&issued.bearer_token).unwrap();
        assert!(identity.grants(&read));
        assert!(!identity.grants(&PermissionName::parse("time.admin").unwrap()));
        assert_eq!(identity.client.as_str(), TEST_CLIENT);

        assert!(matches!(
            issuer.issue(forwarded("time", &["map.read"])),
            Err(InternalTokenError::PermissionOutsideAudience(_))
        ));

        // A correctly signed token that grants another server's permission is refused.
        let identity = issuer
            .create_identity(forwarded("time", &["map.read"]))
            .unwrap();
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = Some(ACCESS_TOKEN_TYPE.to_owned());
        header.kid = Some(signing_key().key_id().to_owned());
        let sign = |claims: &serde_json::Value| {
            encode(
                &header,
                claims,
                &EncodingKey::from_ed_der(&signing_key().private_key_der),
            )
            .unwrap()
        };
        let forged = serde_json::to_value(AccessTokenClaims::from_identity(&identity)).unwrap();
        assert!(matches!(
            verifier.verify(&sign(&forged)),
            Err(InternalTokenError::PermissionOutsideAudience(_))
        ));

        // The grant travels as one RFC 9396 detail; a second detail is refused.
        let granted: serde_json::Value = serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(issued.bearer_token.split('.').nth(1).unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            granted["authorization_details"],
            serde_json::json!([{
                "type": veoveo_mcp_ext_registration::PERMISSIONS_DETAIL_TYPE,
                "actions": ["time.read"]
            }])
        );
        assert_eq!(granted["aud"], resource("time").as_str());
        assert_eq!(granted[INVOCATION_CLAIM]["server"], "time");
        assert_eq!(
            uuid::Uuid::parse_str(granted["jti"].as_str().unwrap())
                .unwrap()
                .get_version_num(),
            7
        );
        assert_eq!(granted["iss"], TEST_ISSUER);
        assert_eq!(granted["client_id"], TEST_CLIENT);
        assert_eq!(granted["act"], serde_json::json!({"sub": TEST_ISSUER}));
        assert!(granted[INVOCATION_CLAIM].is_object());
        let mut doubled = granted.clone();
        let detail = doubled["authorization_details"][0].clone();
        doubled["authorization_details"]
            .as_array_mut()
            .unwrap()
            .push(detail);
        assert!(matches!(
            verifier.verify(&sign(&doubled)),
            Err(InternalTokenError::InconsistentClaims)
        ));

        // RFC 8693: `act` names the issuing Gateway and nothing else.
        for act in [
            serde_json::json!({"sub": "https://other.example"}),
            serde_json::json!({"sub": TEST_ISSUER, "act": {"sub": "agent"}}),
        ] {
            let mut acting = granted.clone();
            acting["act"] = act;
            assert!(verifier.verify(&sign(&acting)).is_err());
        }
        let mut unacted = granted.clone();
        unacted.as_object_mut().unwrap().remove("act");
        assert!(verifier.verify(&sign(&unacted)).is_err());

        // `jti` is a UUIDv7.
        let mut random = granted.clone();
        random["jti"] = serde_json::json!(uuid::Uuid::new_v4().to_string());
        assert!(matches!(
            verifier.verify(&sign(&random)),
            Err(InternalTokenError::InvalidTokenId)
        ));
        // The audience is the server's resource URL, not its slug.
        let mut slugged = granted.clone();
        slugged["aud"] = serde_json::json!("time");
        assert!(verifier.verify(&sign(&slugged)).is_err());
    }

    #[test]
    fn rejects_non_eddsa_trust_keys() {
        assert!(matches!(
            GatewayInternalTrustBundle::from_json(
                r#"{"keys":[{"kty":"oct","k":"c2VjcmV0","alg":"HS256","kid":"bad"}]}"#
            ),
            Err(InternalTokenError::UnsupportedTrustKey)
        ));
    }

    #[test]
    fn internal_token_round_trips_typed_identity() {
        let issued = issuer().issue(forwarded("media", &[])).unwrap();
        assert!(!format!("{issued:?}").contains(&issued.bearer_token));
        let verified = verifier("media").verify(&issued.bearer_token).unwrap();
        assert_eq!(verified.profile.as_str(), "default");
        assert_eq!(verified.server.as_str(), "media");
        assert_eq!(verified.audience, resource("media"));
        assert_eq!(verified.actor.id.as_str(), "https://idp.example.com#user-1");
    }

    #[test]
    fn issuers_are_https_urls() {
        for issuer in ["veoveo-internal", "http://gateway.example"] {
            assert!(matches!(
                GatewayInternalTokenIssuer::new(TokenIssuer::parse(issuer).unwrap(), signing_key()),
                Err(InternalTokenError::InvalidIssuer)
            ));
        }
    }

    #[test]
    fn internal_resource_token_round_trips_without_a_synthetic_profile() {
        let recordings =
            ProtectedResourceId::parse("https://veoveo.example/ingest/recordings").unwrap();
        let issued = issuer()
            .issue_resource(
                recordings.clone(),
                ServerSlug::parse("recording-hub").unwrap(),
                OAuthClientId::parse(TEST_CLIENT).unwrap(),
                principal(),
                authority(),
                Utc::now() + TimeDelta::minutes(5),
            )
            .unwrap();
        assert!(!format!("{issued:?}").contains(&issued.bearer_token));
        let verifier = |server: &str| {
            GatewayInternalResourceTokenVerifier::new(
                TokenIssuer::parse(TEST_ISSUER).unwrap(),
                recordings.clone(),
                ServerSlug::parse(server).unwrap(),
                trust_bundle(),
            )
        };
        let verified = verifier("recording-hub")
            .verify(&issued.bearer_token)
            .unwrap();
        assert_eq!(verified.protected_resource, recordings);
        assert_eq!(verified.server.as_str(), "recording-hub");
        assert_eq!(verified.client.as_str(), TEST_CLIENT);
        assert!(verifier("media").verify(&issued.bearer_token).is_err());
    }

    #[test]
    fn internal_token_rejects_wrong_server_audience() {
        let issued = issuer().issue(forwarded("media", &[])).unwrap();
        let err = verifier("simulation")
            .verify(&issued.bearer_token)
            .expect_err("wrong server audience should be rejected");
        assert!(matches!(err, InternalTokenError::Jwt(_)));
        // The right resource URL with another server's name is refused too.
        let crossed = GatewayInternalTokenVerifier::new(
            TokenIssuer::parse(TEST_ISSUER).unwrap(),
            TokenAudience {
                server: ServerSlug::parse("simulation").unwrap(),
                resource: resource("media"),
            },
            trust_bundle(),
        );
        assert!(matches!(
            crossed.verify(&issued.bearer_token),
            Err(InternalTokenError::AudienceMismatch { .. })
        ));
    }

    fn generated_key() -> GatewayInternalSigningKey {
        GatewayInternalSigningKey::new(ServerKeyPair::generate().unwrap().pkcs8_der()).unwrap()
    }

    #[test]
    fn internal_token_rejects_unknown_key_id() {
        let retired = generated_key();
        let issued = GatewayInternalTokenIssuer::new(
            TokenIssuer::parse(TEST_ISSUER).unwrap(),
            retired.clone(),
        )
        .unwrap()
        .issue(forwarded("media", &[]))
        .unwrap();
        let err = verifier("media")
            .verify(&issued.bearer_token)
            .expect_err("unknown kid should be rejected");
        assert!(matches!(err, InternalTokenError::UnknownKeyId(key) if key == retired.key_id()));
    }

    #[test]
    fn signing_keys_are_named_by_their_thumbprint() {
        let key = signing_key();
        assert_eq!(key.key_id(), key.verification_key().kid().as_str());
        let rotation = Ed25519JwkSet::new(vec![
            key.verification_key(),
            generated_key().verification_key(),
        ])
        .unwrap();
        let bundle = GatewayInternalTrustBundle::from_key_set(&rotation).unwrap();
        assert_eq!(bundle.key_ids().count(), 2);
        assert!(bundle.key_ids().any(|kid| kid == key.key_id()));
        // A trust key named by anything but its thumbprint is refused.
        let x = key.verification_key().key().x();
        assert!(matches!(
            GatewayInternalTrustBundle::from_json(&format!(
                r#"{{"keys":[{{"kty":"OKP","crv":"Ed25519","x":"{x}","alg":"EdDSA","use":"sig","kid":"key-1"}}]}}"#
            )),
            Err(InternalTokenError::KeyIdNotThumbprint(_))
        ));
    }

    #[test]
    fn registration_key_sets_become_trust_bundles() {
        let trust = AccessTokenTrust {
            issuer: TokenIssuer::parse(TEST_ISSUER).unwrap(),
            jwks: Ed25519JwkSet::new(vec![signing_key().verification_key()]).unwrap(),
            resource: resource("time"),
        };
        let issued = issuer().issue(forwarded("time", &[])).unwrap();
        GatewayInternalTokenVerifier::from_access_tokens(
            &trust,
            ServerSlug::parse("time").unwrap(),
        )
        .unwrap()
        .verify(&issued.bearer_token)
        .unwrap();
    }

    #[test]
    fn trust_bundle_rejects_malformed_public_key() {
        assert!(matches!(
            GatewayInternalTrustBundle::from_json(
                r#"{"keys":[{"kty":"OKP","crv":"Ed25519","x":"eA","alg":"EdDSA","use":"sig","kid":"key-1"}]}"#
            ),
            Err(InternalTokenError::UnsupportedTrustKey)
        ));
    }
}
