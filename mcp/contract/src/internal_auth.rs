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
use veoveo_mcp_ext_registration::{Ed25519JwkSet, PermissionsDetail};
use veoveo_types::{
    GroupId, IdentifierError, InvocationAuthority, PermissionName, PrincipalId, RoleId,
};

/// The JOSE `typ` of every token the Gateway forwards: an RFC 9068 JWT access token.
pub const ACCESS_TOKEN_TYPE: &str = "at+jwt";
/// The RFC 9068 `client_id` of forwarded tokens: the Gateway acts as the OAuth client
/// that presents them to the server.
pub const GATEWAY_CLIENT_ID: &str = "veoveo-gateway";
/// Private claim carrying profile, actor, authority and request context.
pub const INVOCATION_CLAIM: &str = "ai.veoveo/invocation";
/// Private claim of HTTP resource tokens.
pub const RESOURCE_INVOCATION_CLAIM: &str = "ai.veoveo/resource-invocation";

pub const GATEWAY_INTERNAL_TOKEN_ISSUER: &str = "veoveo-internal";
pub const DEFAULT_GATEWAY_INTERNAL_SIGNING_KEY_ID: &str = "veoveo-internal-1";

/// Error text for a hosted-server request that arrived without the gateway's
/// internal identity, forwarded bearer, or authenticated HTTP context.
pub const GATEWAY_ROUTING_REQUIRED: &str =
    "This server only accepts requests routed through the Veoveo gateway.";

#[derive(Clone, PartialEq, Eq)]
pub struct GatewayInternalSigningKey {
    key_id: String,
    private_key_der: Vec<u8>,
}

impl GatewayInternalSigningKey {
    pub fn new(
        key_id: impl Into<String>,
        private_key_der: impl Into<Vec<u8>>,
    ) -> Result<Self, InternalTokenError> {
        ensure_jwt_crypto_provider();
        let key_id = validate_key_id(key_id.into())?;
        let private_key_der = private_key_der.into();
        if private_key_der.is_empty() {
            return Err(InternalTokenError::EmptyPrivateKey);
        }
        jsonwebtoken::crypto::sign(
            b"veoveo-internal-signing-key-validation",
            &EncodingKey::from_ed_der(&private_key_der),
            Algorithm::EdDSA,
        )
        .map_err(InternalTokenError::Jwt)?;
        Ok(Self {
            key_id,
            private_key_der,
        })
    }

    pub fn key_id(&self) -> &str {
        &self.key_id
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
            validate_verification_jwk(&jwk)?;
            DecodingKey::from_jwk(&jwk).map_err(InternalTokenError::Jwt)?;
            let key_id = validate_key_id(
                jwk.common
                    .key_id
                    .clone()
                    .ok_or(InternalTokenError::MissingKeyId)?,
            )?;
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

    /// The keys a registration response names in `ai.veoveo/access-tokens`.
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

fn validate_verification_jwk(jwk: &Jwk) -> Result<(), InternalTokenError> {
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
            if parameters.curve == EllipticCurve::Ed25519
                && URL_SAFE_NO_PAD
                    .decode(&parameters.x)
                    .is_ok_and(|key| key.len() == 32) =>
        {
            Ok(())
        }
        _ => Err(InternalTokenError::UnsupportedTrustKey),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GatewayInternalIdentity {
    pub issuer: TokenIssuer,
    pub profile: GatewayProfileId,
    pub server: ServerSlug,
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
    pub fn new(issuer: TokenIssuer, signing_key: GatewayInternalSigningKey) -> Self {
        Self {
            issuer,
            signing_key,
        }
    }

    /// Issue an identity that carries no server permissions, for Gateway-internal paths.
    pub fn issue(
        &self,
        profile: GatewayProfileId,
        server: ServerSlug,
        actor: Principal,
        authority: InvocationAuthority,
        request_context: Option<GatewayRequestContext>,
        expires_at: DateTime<Utc>,
    ) -> Result<IssuedGatewayInternalToken, InternalTokenError> {
        self.issue_granted(
            profile,
            server,
            actor,
            authority,
            BTreeSet::new(),
            request_context,
            expires_at,
        )
    }

    /// Issue an identity carrying the permissions the Gateway granted in `server`'s
    /// namespace. Every permission must belong to that namespace.
    #[allow(clippy::too_many_arguments)]
    pub fn issue_granted(
        &self,
        profile: GatewayProfileId,
        server: ServerSlug,
        actor: Principal,
        authority: InvocationAuthority,
        permissions: BTreeSet<PermissionName>,
        request_context: Option<GatewayRequestContext>,
        expires_at: DateTime<Utc>,
    ) -> Result<IssuedGatewayInternalToken, InternalTokenError> {
        if let Some(foreign) = permissions.iter().find(|p| !p.is_owned_by(&server)) {
            return Err(InternalTokenError::PermissionOutsideAudience(
                foreign.clone(),
            ));
        }
        let identity = self.create_identity(
            profile,
            server,
            actor,
            authority,
            permissions,
            request_context,
            expires_at,
        )?;
        let bearer_token = self.sign(&AccessTokenClaims::from_identity(&identity)?)?;
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

    #[allow(clippy::too_many_arguments)]
    fn create_identity(
        &self,
        profile: GatewayProfileId,
        server: ServerSlug,
        actor: Principal,
        authority: InvocationAuthority,
        permissions: BTreeSet<PermissionName>,
        request_context: Option<GatewayRequestContext>,
        expires_at: DateTime<Utc>,
    ) -> Result<GatewayInternalIdentity, InternalTokenError> {
        ensure_jwt_crypto_provider();
        let now = Utc::now();
        let expires_at = if let Some(context) = &request_context {
            context.validate_for(&actor, &authority)?;
            expires_at.min(context.access_token.expires_at)
        } else {
            expires_at
        };
        if expires_at <= now {
            return Err(InternalTokenError::ExpiredDelegation);
        }
        let jwt_id = JwtId::parse(uuid::Uuid::new_v4().to_string())
            .map_err(InternalTokenError::Identifier)?;
        Ok(GatewayInternalIdentity {
            issuer: self.issuer.clone(),
            profile,
            server,
            actor,
            authority,
            permissions,
            request_context,
            jwt_id,
            issued_at: now,
            not_before: now,
            expires_at,
        })
    }

    pub fn issue_resource(
        &self,
        protected_resource: ProtectedResourceId,
        server: ServerSlug,
        actor: Principal,
        authority: InvocationAuthority,
        expires_at: DateTime<Utc>,
    ) -> Result<IssuedGatewayInternalResourceToken, InternalTokenError> {
        ensure_jwt_crypto_provider();
        let now = Utc::now();
        if expires_at <= now {
            return Err(InternalTokenError::ExpiredDelegation);
        }
        let jwt_id = JwtId::parse(uuid::Uuid::new_v4().to_string())
            .map_err(InternalTokenError::Identifier)?;
        let identity = GatewayInternalResourceIdentity {
            issuer: self.issuer.clone(),
            protected_resource,
            server,
            actor,
            authority,
            jwt_id,
            issued_at: now,
            not_before: now,
            expires_at,
        };
        let bearer_token = self.sign(&ResourceAccessTokenClaims::from_identity(&identity)?)?;
        Ok(IssuedGatewayInternalResourceToken {
            bearer_token,
            identity,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GatewayInternalResourceIdentity {
    pub issuer: TokenIssuer,
    pub protected_resource: ProtectedResourceId,
    pub server: ServerSlug,
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
    audience: ServerSlug,
    trust_bundle: GatewayInternalTrustBundle,
}

impl GatewayInternalResourceTokenVerifier {
    pub fn new(
        issuer: TokenIssuer,
        audience: ServerSlug,
        trust_bundle: GatewayInternalTrustBundle,
    ) -> Self {
        Self {
            issuer,
            audience,
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
            std::slice::from_ref(&self.audience),
            &self.trust_bundle,
        )?;
        let server = claims.standard.audience()?;
        if server != self.audience {
            return Err(InternalTokenError::AudienceMismatch {
                expected: self.audience.clone(),
                actual: server,
            });
        }
        let invocation = claims.invocation;
        claims.standard.check_actor(&invocation.actor)?;
        Ok(GatewayInternalResourceIdentity {
            issuer: claims.standard.iss,
            protected_resource: invocation.protected_resource,
            server,
            actor: invocation.actor,
            authority: invocation.authority,
            jwt_id: claims.standard.jti,
            issued_at: timestamp_to_datetime(claims.standard.iat, "iat")?,
            not_before: timestamp_to_datetime(claims.standard.nbf, "nbf")?,
            expires_at: timestamp_to_datetime(claims.standard.exp, "exp")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct GatewayInternalTokenVerifier {
    issuer: TokenIssuer,
    audiences: Vec<ServerSlug>,
    trust_bundle: GatewayInternalTrustBundle,
}

impl GatewayInternalTokenVerifier {
    /// Verify tokens audienced to exactly one server (the common upstream case).
    pub fn new(
        issuer: TokenIssuer,
        audience: ServerSlug,
        trust_bundle: GatewayInternalTrustBundle,
    ) -> Self {
        Self {
            issuer,
            audiences: vec![audience],
            trust_bundle,
        }
    }

    /// Verify tokens audienced to any of `audiences`.
    ///
    /// The shared artifact plane uses this: a domain server forwards the
    /// gateway token it received (audienced to *that* server) on the principal's
    /// behalf, so the plane must accept the set of known upstream slugs rather
    /// than a single audience. The gateway remains the only minter, so trust is
    /// unchanged; only which forwarded audiences are honored widens.
    pub fn new_for_audiences(
        issuer: TokenIssuer,
        audiences: Vec<ServerSlug>,
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
        decode_access_token(
            bearer_token,
            &self.issuer,
            &self.audiences,
            &self.trust_bundle,
        )
    }

    fn identity_from_claims(
        &self,
        claims: AccessTokenClaims,
    ) -> Result<GatewayInternalIdentity, InternalTokenError> {
        let server = claims.standard.audience()?;
        if !self.audiences.contains(&server) {
            return Err(InternalTokenError::AudienceMismatch {
                expected: self
                    .audiences
                    .first()
                    .cloned()
                    .unwrap_or_else(|| server.clone()),
                actual: server,
            });
        }
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
            if claims.standard.exp > context.access_token.expires_at.timestamp() {
                return Err(InternalTokenError::InvalidRequestContext);
            }
        }
        Ok(GatewayInternalIdentity {
            issuer: claims.standard.iss,
            profile: invocation.profile,
            server,
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
    audiences: &[ServerSlug],
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
    let audiences: Vec<&str> = audiences.iter().map(ServerSlug::as_str).collect();
    validation.set_audience(&audiences);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub", "nbf"]);
    Ok(decode::<T>(bearer_token, &decoding_key, &validation)
        .map_err(InternalTokenError::Jwt)?
        .claims)
}

fn ensure_jwt_crypto_provider() {
    let _ = jsonwebtoken::crypto::aws_lc::DEFAULT_PROVIDER.install_default();
}

/// RFC 9068 §2.2 claims of every token the Gateway forwards. `sub`, `groups`, `roles`
/// and `scope` restate the embedded actor for standard resource-server libraries; a
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    groups: BTreeSet<GroupId>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    roles: BTreeSet<RoleId>,
}

impl StandardClaims {
    #[allow(clippy::too_many_arguments)]
    fn new(
        issuer: &TokenIssuer,
        actor: &Principal,
        audience: &ServerSlug,
        jwt_id: &JwtId,
        issued_at: DateTime<Utc>,
        not_before: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Result<Self, InternalTokenError> {
        Ok(Self {
            iss: issuer.clone(),
            sub: actor.id.as_str().to_owned(),
            aud: audience.as_str().to_owned(),
            client_id: gateway_client_id()?,
            exp: expires_at.timestamp(),
            nbf: not_before.timestamp(),
            iat: issued_at.timestamp(),
            jti: jwt_id.clone(),
            scope: scope_claim(actor),
            groups: actor.groups.clone(),
            roles: actor.roles.clone(),
        })
    }

    fn audience(&self) -> Result<ServerSlug, InternalTokenError> {
        ServerSlug::parse(self.aud.clone()).map_err(InternalTokenError::Identifier)
    }

    fn check_actor(&self, actor: &Principal) -> Result<(), InternalTokenError> {
        if PrincipalId::parse(self.sub.clone()).map_err(InternalTokenError::Identifier)? != actor.id
        {
            return Err(InternalTokenError::SubjectPrincipalMismatch);
        }
        if self.client_id != gateway_client_id()?
            || self.groups != actor.groups
            || self.roles != actor.roles
            || self.scope != scope_claim(actor)
        {
            return Err(InternalTokenError::InconsistentClaims);
        }
        Ok(())
    }
}

fn gateway_client_id() -> Result<OAuthClientId, InternalTokenError> {
    OAuthClientId::parse(GATEWAY_CLIENT_ID).map_err(InternalTokenError::Identifier)
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
    #[serde(rename = "ai.veoveo/invocation")]
    invocation: Invocation,
}

/// The `ai.veoveo/invocation` claim: who acts, under which authority and request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Invocation {
    profile: GatewayProfileId,
    actor: Principal,
    authority: InvocationAuthority,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_context: Option<GatewayRequestContext>,
}

impl AccessTokenClaims {
    fn from_identity(identity: &GatewayInternalIdentity) -> Result<Self, InternalTokenError> {
        Ok(Self {
            standard: StandardClaims::new(
                &identity.issuer,
                &identity.actor,
                &identity.server,
                &identity.jwt_id,
                identity.issued_at,
                identity.not_before,
                identity.expires_at,
            )?,
            authorization_details: (!identity.permissions.is_empty())
                .then(|| PermissionsDetail::new(identity.permissions.clone()))
                .into_iter()
                .collect(),
            invocation: Invocation {
                profile: identity.profile.clone(),
                actor: identity.actor.clone(),
                authority: identity.authority.clone(),
                request_context: identity.request_context.clone(),
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ResourceAccessTokenClaims {
    #[serde(flatten)]
    standard: StandardClaims,
    #[serde(rename = "ai.veoveo/resource-invocation")]
    invocation: ResourceInvocation,
}

/// The `ai.veoveo/resource-invocation` claim of an HTTP resource token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceInvocation {
    protected_resource: ProtectedResourceId,
    actor: Principal,
    authority: InvocationAuthority,
}

impl ResourceAccessTokenClaims {
    fn from_identity(
        identity: &GatewayInternalResourceIdentity,
    ) -> Result<Self, InternalTokenError> {
        Ok(Self {
            standard: StandardClaims::new(
                &identity.issuer,
                &identity.actor,
                &identity.server,
                &identity.jwt_id,
                identity.issued_at,
                identity.not_before,
                identity.expires_at,
            )?,
            invocation: ResourceInvocation {
                protected_resource: identity.protected_resource.clone(),
                actor: identity.actor.clone(),
                authority: identity.authority.clone(),
            },
        })
    }
}

#[derive(Debug)]
pub enum InternalTokenError {
    EmptyPrivateKey,
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
    /// Standard claims disagree with the embedded actor, or the token carries more
    /// than one permissions detail.
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
        PUBLIC_KEY_X, authority, principal, signing_key, trust_bundle,
    };

    #[test]
    fn upload_assertions_require_dedicated_audience_and_signed_policy_binding() {
        let issuer = GatewayInternalTokenIssuer::new(
            TokenIssuer::parse("veoveo-internal").unwrap(),
            signing_key("key-1"),
        );
        let verifier = GatewayInternalTokenVerifier::new(
            TokenIssuer::parse("veoveo-internal").unwrap(),
            ServerSlug::parse(veoveo_artifact_contract::ARTIFACT_UPLOAD_AUDIENCE).unwrap(),
            trust_bundle("key-1"),
        );
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
        let ordinary = issuer
            .issue(
                GatewayProfileId::parse("default").unwrap(),
                ServerSlug::parse(veoveo_artifact_contract::ARTIFACT_UPLOAD_AUDIENCE).unwrap(),
                principal(),
                authority(),
                None,
                Utc::now() + TimeDelta::minutes(1),
            )
            .unwrap();
        assert!(
            verifier
                .verify_artifact_upload(&ordinary.bearer_token)
                .is_err(),
            "a token without a signed policy binding was accepted"
        );
        let domain = GatewayInternalTokenVerifier::new(
            TokenIssuer::parse("veoveo-internal").unwrap(),
            ServerSlug::parse("media").unwrap(),
            trust_bundle("key-1"),
        );
        assert!(domain.verify(&token).is_err());
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
        let issuer = GatewayInternalTokenIssuer::new(
            TokenIssuer::parse("veoveo-internal").unwrap(),
            signing_key("key-1"),
        );
        let verifier = GatewayInternalTokenVerifier::new(
            TokenIssuer::parse("veoveo-internal").unwrap(),
            ServerSlug::parse("time").unwrap(),
            trust_bundle("key-1"),
        );
        let time = ServerSlug::parse("time").unwrap();
        let read = PermissionName::parse("time.read").unwrap();
        let expires = Utc::now() + TimeDelta::minutes(1);
        let issued = issuer
            .issue_granted(
                GatewayProfileId::parse("default").unwrap(),
                time.clone(),
                principal(),
                authority(),
                std::collections::BTreeSet::from([read.clone()]),
                None,
                expires,
            )
            .unwrap();
        let identity = verifier.verify(&issued.bearer_token).unwrap();
        assert!(identity.grants(&read));
        assert!(!identity.grants(&PermissionName::parse("time.admin").unwrap()));

        let foreign =
            std::collections::BTreeSet::from([PermissionName::parse("map.read").unwrap()]);
        assert!(matches!(
            issuer.issue_granted(
                GatewayProfileId::parse("default").unwrap(),
                time.clone(),
                principal(),
                authority(),
                foreign.clone(),
                None,
                expires,
            ),
            Err(InternalTokenError::PermissionOutsideAudience(_))
        ));

        // A correctly signed token that grants another server's permission is refused.
        let identity = issuer
            .create_identity(
                GatewayProfileId::parse("default").unwrap(),
                time,
                principal(),
                authority(),
                foreign,
                None,
                expires,
            )
            .unwrap();
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = Some(ACCESS_TOKEN_TYPE.to_owned());
        header.kid = Some("key-1".to_owned());
        let sign = |claims: &serde_json::Value| {
            encode(
                &header,
                claims,
                &EncodingKey::from_ed_der(&signing_key("key-1").private_key_der),
            )
            .unwrap()
        };
        let forged =
            serde_json::to_value(AccessTokenClaims::from_identity(&identity).unwrap()).unwrap();
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
        assert_eq!(granted["aud"], "time");
        assert_eq!(granted["client_id"], GATEWAY_CLIENT_ID);
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
        let issuer = GatewayInternalTokenIssuer::new(
            TokenIssuer::parse("veoveo-internal").unwrap(),
            signing_key("key-1"),
        );
        let issued = issuer
            .issue(
                GatewayProfileId::parse("default").unwrap(),
                ServerSlug::parse("media").unwrap(),
                principal(),
                authority(),
                None,
                Utc::now() + TimeDelta::minutes(5),
            )
            .unwrap();
        assert!(!format!("{issued:?}").contains(&issued.bearer_token));

        let verified = GatewayInternalTokenVerifier::new(
            TokenIssuer::parse("veoveo-internal").unwrap(),
            ServerSlug::parse("media").unwrap(),
            trust_bundle("key-1"),
        )
        .verify(&issued.bearer_token)
        .unwrap();

        assert_eq!(verified.profile.as_str(), "default");
        assert_eq!(verified.server.as_str(), "media");
        assert_eq!(verified.actor.id.as_str(), "https://idp.example.com#user-1");
    }

    #[test]
    fn internal_resource_token_round_trips_without_a_synthetic_profile() {
        let issuer = GatewayInternalTokenIssuer::new(
            TokenIssuer::parse("veoveo-internal").unwrap(),
            signing_key("key-1"),
        );
        let issued = issuer
            .issue_resource(
                ProtectedResourceId::parse("https://veoveo.example/ingest/recordings").unwrap(),
                ServerSlug::parse("recording-hub").unwrap(),
                principal(),
                authority(),
                Utc::now() + TimeDelta::minutes(5),
            )
            .unwrap();
        assert!(!format!("{issued:?}").contains(&issued.bearer_token));

        let verified = GatewayInternalResourceTokenVerifier::new(
            TokenIssuer::parse("veoveo-internal").unwrap(),
            ServerSlug::parse("recording-hub").unwrap(),
            trust_bundle("key-1"),
        )
        .verify(&issued.bearer_token)
        .unwrap();
        assert_eq!(
            verified.protected_resource.as_str(),
            "https://veoveo.example/ingest/recordings"
        );
        assert_eq!(verified.server.as_str(), "recording-hub");
    }

    #[test]
    fn internal_token_rejects_wrong_server_audience() {
        let issuer = GatewayInternalTokenIssuer::new(
            TokenIssuer::parse("veoveo-internal").unwrap(),
            signing_key("key-1"),
        );
        let issued = issuer
            .issue(
                GatewayProfileId::parse("default").unwrap(),
                ServerSlug::parse("media").unwrap(),
                principal(),
                authority(),
                None,
                Utc::now() + TimeDelta::minutes(5),
            )
            .unwrap();

        let err = GatewayInternalTokenVerifier::new(
            TokenIssuer::parse("veoveo-internal").unwrap(),
            ServerSlug::parse("simulation").unwrap(),
            trust_bundle("key-1"),
        )
        .verify(&issued.bearer_token)
        .expect_err("wrong server audience should be rejected");

        assert!(matches!(err, InternalTokenError::Jwt(_)));
    }

    #[test]
    fn internal_token_rejects_unknown_key_id() {
        let issuer = GatewayInternalTokenIssuer::new(
            TokenIssuer::parse("veoveo-internal").unwrap(),
            signing_key("retired-key"),
        );
        let issued = issuer
            .issue(
                GatewayProfileId::parse("default").unwrap(),
                ServerSlug::parse("media").unwrap(),
                principal(),
                authority(),
                None,
                Utc::now() + TimeDelta::minutes(5),
            )
            .unwrap();

        let err = GatewayInternalTokenVerifier::new(
            TokenIssuer::parse("veoveo-internal").unwrap(),
            ServerSlug::parse("media").unwrap(),
            trust_bundle("active-key"),
        )
        .verify(&issued.bearer_token)
        .expect_err("unknown kid should be rejected");

        assert!(matches!(err, InternalTokenError::UnknownKeyId(key) if key == "retired-key"));
    }

    #[test]
    fn trust_bundle_accepts_rotation_set() {
        let bundle = GatewayInternalTrustBundle::from_json(&format!(
            r#"{{"keys":[
                {{"kty":"OKP","crv":"Ed25519","x":"{PUBLIC_KEY_X}","alg":"EdDSA","use":"sig","kid":"key-1"}},
                {{"kty":"OKP","crv":"Ed25519","x":"{PUBLIC_KEY_X}","alg":"EdDSA","use":"sig","kid":"key-2"}}
            ]}}"#
        ))
        .unwrap();

        assert_eq!(bundle.key_ids().collect::<Vec<_>>(), vec!["key-1", "key-2"]);
    }

    #[test]
    fn registration_key_sets_become_trust_bundles() {
        let keys: Ed25519JwkSet = serde_json::from_value(serde_json::json!({"keys": [
            {"kty": "OKP", "crv": "Ed25519", "x": PUBLIC_KEY_X, "alg": "EdDSA", "use": "sig", "kid": "key-1"}
        ]}))
        .unwrap();
        let bundle = GatewayInternalTrustBundle::from_key_set(&keys).unwrap();
        let issued = GatewayInternalTokenIssuer::new(
            TokenIssuer::parse("veoveo-internal").unwrap(),
            signing_key("key-1"),
        )
        .issue(
            GatewayProfileId::parse("default").unwrap(),
            ServerSlug::parse("time").unwrap(),
            principal(),
            authority(),
            None,
            Utc::now() + TimeDelta::minutes(1),
        )
        .unwrap();
        GatewayInternalTokenVerifier::new(
            TokenIssuer::parse("veoveo-internal").unwrap(),
            ServerSlug::parse("time").unwrap(),
            bundle,
        )
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
