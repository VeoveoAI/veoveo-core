use std::{collections::BTreeSet, fmt};

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayControlPlaneError {
    DuplicateRole(veoveo_types::RoleId),
    UnknownGrantRole(veoveo_types::RoleId),
    UnknownGrantTenant {
        role: veoveo_types::RoleId,
        tenant: TenantId,
    },
    ConflictingPermissionOverride(veoveo_types::PermissionName),
    UnknownRegistrationSecret(SecretReferenceId),
    RegistrationSecretMismatch(SecretReferenceId),
    PolicyRuleServerAction {
        policy: PolicyVersion,
        rule: PolicyRuleId,
        action: veoveo_types::ActionName,
    },
    PolicyRuleUnsupportedAction {
        policy: PolicyVersion,
        rule: PolicyRuleId,
        action: veoveo_types::ActionName,
    },
    CatalogAdmission(veoveo_types::ExtensionError),
    DuplicateIdentityProvider(IdentityProviderId),
    DuplicateAuthorizationServer(AuthorizationServerId),
    DuplicateProfile(GatewayProfileId),
    InvalidArtifactUploadPolicy(GatewayProfileId),
    DuplicateProtectedResource(ProtectedResourceId),
    DuplicatePolicy(PolicyVersion),
    DuplicateDataLabel(DataLabelId),
    DuplicateTenant(TenantId),
    DuplicateWorkContext(WorkContextId),
    InvalidWorkContext {
        context: WorkContextId,
        reason: String,
    },
    InvalidBranding {
        field: &'static str,
        reason: String,
    },
    DuplicateSecret(SecretReferenceId),
    DuplicateOAuthClient(OAuthClientId),
    DuplicateOidcClient(OidcClientRegistrationId),
    DuplicatePolicyRule {
        policy: PolicyVersion,
        rule: PolicyRuleId,
    },
    UnknownPolicyRuleProfile {
        policy: PolicyVersion,
        rule: PolicyRuleId,
        profile: GatewayProfileId,
    },
    UnknownPolicyRuleProtectedResource {
        policy: PolicyVersion,
        rule: PolicyRuleId,
        resource: ProtectedResourceId,
    },
    UnknownPolicyRuleDataLabel {
        policy: PolicyVersion,
        rule: PolicyRuleId,
        label: DataLabelId,
    },
    UnknownPolicyRuleTenant {
        policy: PolicyVersion,
        rule: PolicyRuleId,
        tenant: TenantId,
    },
    UnknownPolicy {
        profile: GatewayProfileId,
        policy_version: PolicyVersion,
    },
    UnknownIdentityProvider {
        profile: GatewayProfileId,
        identity_provider: IdentityProviderId,
    },
    UnknownAuthorizationServer {
        profile: GatewayProfileId,
        authorization_server: AuthorizationServerId,
    },
    UnknownAuthorizationServerIdentityProvider {
        authorization_server: AuthorizationServerId,
        identity_provider: IdentityProviderId,
    },
    UnknownIdentityProviderMappedTenant {
        identity_provider: IdentityProviderId,
        tenant: TenantId,
    },
    UnknownAuthorizationServerSigningKey {
        authorization_server: AuthorizationServerId,
        secret: SecretReferenceId,
    },
    AuthorizationServerSigningKeyPurposeMismatch {
        authorization_server: AuthorizationServerId,
        secret: SecretReferenceId,
        purpose: SecretPurpose,
    },
    MissingAuthModes {
        profile: GatewayProfileId,
    },
    MissingIdentityProviderEndpoint {
        profile: GatewayProfileId,
        identity_provider: IdentityProviderId,
        endpoint: IdentityProviderEndpoint,
    },
    MissingAuthorizationServerEndpoint {
        profile: GatewayProfileId,
        authorization_server: AuthorizationServerId,
        endpoint: AuthorizationServerEndpoint,
    },
    UnknownSecretOwnerProfile {
        secret: SecretReferenceId,
        profile: GatewayProfileId,
    },
    UnknownSecretOwnerTenant {
        secret: SecretReferenceId,
        tenant: TenantId,
    },
    UnknownOAuthClientAuthorizationServer {
        client: OAuthClientId,
        authorization_server: AuthorizationServerId,
    },
    UnknownOAuthClientWorkContext {
        client: OAuthClientId,
        context: WorkContextId,
    },
    OAuthClientInvocationModeMismatch {
        client: OAuthClientId,
        mode: veoveo_types::InvocationMode,
    },
    UnknownOAuthClientResource {
        client: OAuthClientId,
        resource: ProtectedResourceId,
    },
    UnknownOAuthClientSecret {
        client: OAuthClientId,
        secret: SecretReferenceId,
    },
    OAuthClientSecretPurposeMismatch {
        client: OAuthClientId,
        secret: SecretReferenceId,
        purpose: SecretPurpose,
    },
    OAuthClientResourceAuthorizationServerMismatch {
        client: OAuthClientId,
        resource: ProtectedResourceId,
        client_authorization_server: AuthorizationServerId,
        resource_authorization_server: AuthorizationServerId,
    },
    OAuthClientWithoutAllowedResources(OAuthClientId),
    OAuthClientWithoutGrantTypes(OAuthClientId),
    OAuthClientWithoutAuthMethods(OAuthClientId),
    OAuthClientMissingRedirectUris {
        client: OAuthClientId,
        grant_type: OAuthGrantType,
    },
    OAuthClientPublicClientCredentials(OAuthClientId),
    OAuthClientMissingCredentialSecret {
        client: OAuthClientId,
        auth_method: OAuthClientAuthMethod,
    },
    OAuthClientMissingJwks {
        client: OAuthClientId,
        auth_method: OAuthClientAuthMethod,
    },
    OAuthClientMissingAllowedScope {
        client: OAuthClientId,
        resource: ProtectedResourceId,
        scope: ScopeName,
    },
    OAuthClientFullMcpWithCompatibility {
        client: OAuthClientId,
    },
    OAuthClientUnsupportedAuthConfiguration {
        client: OAuthClientId,
        grant_types: BTreeSet<OAuthGrantType>,
        auth_methods: BTreeSet<OAuthClientAuthMethod>,
    },
    MissingOAuthClientForAuthMode {
        profile: GatewayProfileId,
        auth_mode: AuthMode,
    },
    UnknownOidcClientIdentityProvider {
        client: OidcClientRegistrationId,
        identity_provider: IdentityProviderId,
    },
    UnknownOidcClientAuthorizationServer {
        client: OidcClientRegistrationId,
        authorization_server: AuthorizationServerId,
    },
    OidcClientAuthorizationServerIdentityProviderMismatch {
        client: OidcClientRegistrationId,
        identity_provider: IdentityProviderId,
        authorization_server: AuthorizationServerId,
        authorization_server_identity_provider: Option<IdentityProviderId>,
    },
    OidcClientWithoutAllowedResources(OidcClientRegistrationId),
    UnknownOidcClientResource {
        client: OidcClientRegistrationId,
        resource: ProtectedResourceId,
    },
    OidcClientResourceAuthorizationServerMismatch {
        client: OidcClientRegistrationId,
        resource: ProtectedResourceId,
        client_authorization_server: AuthorizationServerId,
        resource_authorization_server: AuthorizationServerId,
    },
    OidcClientResourceIdentityProviderMismatch {
        client: OidcClientRegistrationId,
        resource: ProtectedResourceId,
        client_identity_provider: IdentityProviderId,
        resource_identity_provider: IdentityProviderId,
    },
    UnknownOidcClientSecret {
        client: OidcClientRegistrationId,
        secret: SecretReferenceId,
    },
    OidcClientSecretPurposeMismatch {
        client: OidcClientRegistrationId,
        secret: SecretReferenceId,
        purpose: SecretPurpose,
    },
    OidcClientMissingOpenIdScope(OidcClientRegistrationId),
    MissingOidcClientForProfile {
        profile: GatewayProfileId,
        identity_provider: IdentityProviderId,
        authorization_server: AuthorizationServerId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityProviderEndpoint {
    Authorization,
    Token,
    EnterpriseManagedAuthorization,
}

impl IdentityProviderEndpoint {
    fn description(self) -> &'static str {
        match self {
            Self::Authorization => "authorization endpoint",
            Self::Token => "token endpoint",
            Self::EnterpriseManagedAuthorization => "enterprise-managed authorization endpoint",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationServerEndpoint {
    Authorization,
}

impl AuthorizationServerEndpoint {
    fn description(self) -> &'static str {
        match self {
            Self::Authorization => "authorization endpoint",
        }
    }
}

impl fmt::Display for GatewayControlPlaneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CatalogAdmission(error) => fmt::Display::fmt(error, f),
            Self::DuplicateRole(role) => write!(f, "duplicate installation role `{role}`"),
            Self::UnknownGrantRole(role) => write!(f, "grant names unknown role `{role}`"),
            Self::UnknownGrantTenant { role, tenant } => {
                write!(f, "grant of role `{role}` names unknown tenant `{tenant}`")
            }
            Self::ConflictingPermissionOverride(permission) => {
                write!(f, "permission `{permission}` has conflicting overrides")
            }
            Self::UnknownRegistrationSecret(secret) => {
                write!(f, "registration secret `{secret}` is not declared")
            }
            Self::RegistrationSecretMismatch(secret) => write!(
                f,
                "registration secret `{secret}` must be Gateway-owned with the registration_secret purpose"
            ),
            Self::PolicyRuleServerAction {
                policy,
                rule,
                action,
            } => write!(
                f,
                "policy `{policy}` rule `{rule}` names server action `{action}`; roles grant server actions"
            ),
            Self::PolicyRuleUnsupportedAction {
                policy,
                rule,
                action,
            } => write!(
                f,
                "policy `{policy}` rule `{rule}` uses action `{action}` with unsupported selectors"
            ),
            Self::DuplicateIdentityProvider(identity_provider) => {
                write!(f, "duplicate identity provider `{identity_provider}`")
            }
            Self::DuplicateAuthorizationServer(authorization_server) => {
                write!(
                    f,
                    "duplicate resource authorization server `{authorization_server}`"
                )
            }
            Self::DuplicateProfile(profile) => write!(f, "duplicate gateway profile `{profile}`"),
            Self::InvalidArtifactUploadPolicy(profile) => {
                write!(f, "invalid artifact upload policy for profile `{profile}`")
            }
            Self::DuplicateProtectedResource(resource) => {
                write!(f, "duplicate protected resource `{resource}`")
            }
            Self::DuplicatePolicy(policy) => write!(f, "duplicate policy version `{policy}`"),
            Self::DuplicateDataLabel(label) => write!(f, "duplicate data label `{label}`"),
            Self::DuplicateTenant(tenant) => write!(f, "duplicate tenant `{tenant}`"),
            Self::DuplicateWorkContext(context) => {
                write!(f, "duplicate Work Context `{context}`")
            }
            Self::InvalidWorkContext { context, reason } => {
                write!(f, "invalid Work Context `{context}`: {reason}")
            }
            Self::InvalidBranding { field, reason } => {
                write!(f, "installation branding `{field}` {reason}")
            }
            Self::DuplicateSecret(secret) => write!(f, "duplicate secret reference `{secret}`"),
            Self::DuplicateOAuthClient(client) => {
                write!(f, "duplicate OAuth client registration `{client}`")
            }
            Self::DuplicateOidcClient(client) => {
                write!(f, "duplicate OIDC client registration `{client}`")
            }
            Self::DuplicatePolicyRule { policy, rule } => {
                write!(f, "duplicate policy rule `{rule}` in policy `{policy}`")
            }
            Self::UnknownPolicyRuleProfile {
                policy,
                rule,
                profile,
            } => write!(
                f,
                "policy `{policy}` rule `{rule}` references unknown profile `{profile}`"
            ),
            Self::UnknownPolicyRuleProtectedResource {
                policy,
                rule,
                resource,
            } => write!(
                f,
                "policy `{policy}` rule `{rule}` references unknown protected resource `{resource}`"
            ),
            Self::UnknownPolicyRuleDataLabel {
                policy,
                rule,
                label,
            } => write!(
                f,
                "policy `{policy}` rule `{rule}` references unknown data label `{label}`"
            ),
            Self::UnknownPolicyRuleTenant {
                policy,
                rule,
                tenant,
            } => write!(
                f,
                "policy `{policy}` rule `{rule}` references unknown tenant `{tenant}`"
            ),
            Self::UnknownPolicy {
                profile,
                policy_version,
            } => write!(
                f,
                "gateway profile `{profile}` references unknown policy `{policy_version}`"
            ),
            Self::UnknownIdentityProvider {
                profile,
                identity_provider,
            } => write!(
                f,
                "gateway profile `{profile}` references unknown identity provider `{identity_provider}`"
            ),
            Self::UnknownAuthorizationServer {
                profile,
                authorization_server,
            } => write!(
                f,
                "gateway profile `{profile}` references unknown resource authorization server `{authorization_server}`"
            ),
            Self::UnknownAuthorizationServerIdentityProvider {
                authorization_server,
                identity_provider,
            } => write!(
                f,
                "resource authorization server `{authorization_server}` references unknown identity provider `{identity_provider}`"
            ),
            Self::UnknownIdentityProviderMappedTenant {
                identity_provider,
                tenant,
            } => write!(
                f,
                "identity provider `{identity_provider}` claim mapping references unknown tenant `{tenant}`"
            ),
            Self::UnknownAuthorizationServerSigningKey {
                authorization_server,
                secret,
            } => write!(
                f,
                "resource authorization server `{authorization_server}` references unknown signing key secret `{secret}`"
            ),
            Self::AuthorizationServerSigningKeyPurposeMismatch {
                authorization_server,
                secret,
                purpose,
            } => write!(
                f,
                "resource authorization server `{authorization_server}` references signing key secret `{secret}` with invalid purpose `{purpose:?}`"
            ),
            Self::MissingAuthModes { profile } => {
                write!(f, "gateway profile `{profile}` declares no auth modes")
            }
            Self::MissingIdentityProviderEndpoint {
                profile,
                identity_provider,
                endpoint,
            } => write!(
                f,
                "gateway profile `{profile}` requires {} on identity provider `{identity_provider}`",
                endpoint.description()
            ),
            Self::MissingAuthorizationServerEndpoint {
                profile,
                authorization_server,
                endpoint,
            } => write!(
                f,
                "gateway profile `{profile}` requires {} on resource authorization server `{authorization_server}`",
                endpoint.description()
            ),
            Self::UnknownSecretOwnerProfile { secret, profile } => write!(
                f,
                "secret reference `{secret}` is owned by unknown gateway profile `{profile}`"
            ),
            Self::UnknownSecretOwnerTenant { secret, tenant } => write!(
                f,
                "secret reference `{secret}` is owned by unknown tenant `{tenant}`"
            ),
            Self::UnknownOAuthClientAuthorizationServer {
                client,
                authorization_server,
            } => write!(
                f,
                "OAuth client `{client}` references unknown resource authorization server `{authorization_server}`"
            ),
            Self::UnknownOAuthClientWorkContext { client, context } => write!(
                f,
                "OAuth client `{client}` references unknown Work Context `{context}`"
            ),
            Self::OAuthClientInvocationModeMismatch { client, mode } => write!(
                f,
                "OAuth client `{client}` grant types cannot establish `{mode:?}` invocation authority"
            ),
            Self::UnknownOAuthClientResource { client, resource } => write!(
                f,
                "OAuth client `{client}` references unknown protected resource `{resource}`"
            ),
            Self::UnknownOAuthClientSecret { client, secret } => write!(
                f,
                "OAuth client `{client}` references unknown secret `{secret}`"
            ),
            Self::OAuthClientSecretPurposeMismatch {
                client,
                secret,
                purpose,
            } => write!(
                f,
                "OAuth client `{client}` references secret `{secret}` with invalid purpose `{purpose:?}`"
            ),
            Self::OAuthClientResourceAuthorizationServerMismatch {
                client,
                resource,
                client_authorization_server,
                resource_authorization_server,
            } => write!(
                f,
                "OAuth client `{client}` uses resource authorization server `{client_authorization_server}` but protected resource `{resource}` uses `{resource_authorization_server}`"
            ),
            Self::OAuthClientWithoutAllowedResources(client) => {
                write!(
                    f,
                    "OAuth client `{client}` does not allow any protected resource"
                )
            }
            Self::OAuthClientWithoutGrantTypes(client) => {
                write!(f, "OAuth client `{client}` does not declare any grant type")
            }
            Self::OAuthClientWithoutAuthMethods(client) => {
                write!(
                    f,
                    "OAuth client `{client}` does not declare any auth method"
                )
            }
            Self::OAuthClientMissingRedirectUris { client, grant_type } => write!(
                f,
                "OAuth client `{client}` grant `{grant_type:?}` requires at least one redirect URI"
            ),
            Self::OAuthClientPublicClientCredentials(client) => write!(
                f,
                "OAuth client `{client}` cannot use unauthenticated public client credentials"
            ),
            Self::OAuthClientMissingCredentialSecret {
                client,
                auth_method,
            } => write!(
                f,
                "OAuth client `{client}` auth method `{auth_method:?}` requires a credential secret reference"
            ),
            Self::OAuthClientMissingJwks {
                client,
                auth_method,
            } => write!(
                f,
                "OAuth client `{client}` auth method `{auth_method:?}` requires a trusted JWKS source"
            ),
            Self::OAuthClientMissingAllowedScope {
                client,
                resource,
                scope,
            } => write!(
                f,
                "OAuth client `{client}` allows protected resource `{resource}` but does not allow required scope `{scope}`"
            ),
            Self::OAuthClientFullMcpWithCompatibility { client } => write!(
                f,
                "OAuth client `{client}` uses full_mcp surface but declares compatibility helpers or direct task adaptation"
            ),
            Self::OAuthClientUnsupportedAuthConfiguration {
                client,
                grant_types,
                auth_methods,
            } => write!(
                f,
                "OAuth client `{client}` uses unsupported grant/auth method combination: grants `{grant_types:?}`, auth methods `{auth_methods:?}`"
            ),
            Self::MissingOAuthClientForAuthMode { profile, auth_mode } => write!(
                f,
                "gateway profile `{profile}` advertises auth mode `{auth_mode:?}` without a matching OAuth client registration"
            ),
            Self::UnknownOidcClientIdentityProvider {
                client,
                identity_provider,
            } => write!(
                f,
                "OIDC client `{client}` references unknown identity provider `{identity_provider}`"
            ),
            Self::UnknownOidcClientAuthorizationServer {
                client,
                authorization_server,
            } => write!(
                f,
                "OIDC client `{client}` references unknown resource authorization server `{authorization_server}`"
            ),
            Self::OidcClientAuthorizationServerIdentityProviderMismatch {
                client,
                identity_provider,
                authorization_server,
                authorization_server_identity_provider,
            } => write!(
                f,
                "OIDC client `{client}` uses identity provider `{identity_provider}` but resource authorization server `{authorization_server}` is bound to `{authorization_server_identity_provider:?}`"
            ),
            Self::OidcClientWithoutAllowedResources(client) => {
                write!(
                    f,
                    "OIDC client `{client}` does not allow any protected resource"
                )
            }
            Self::UnknownOidcClientResource { client, resource } => write!(
                f,
                "OIDC client `{client}` references unknown protected resource `{resource}`"
            ),
            Self::OidcClientResourceAuthorizationServerMismatch {
                client,
                resource,
                client_authorization_server,
                resource_authorization_server,
            } => write!(
                f,
                "OIDC client `{client}` uses resource authorization server `{client_authorization_server}` but protected resource `{resource}` uses `{resource_authorization_server}`"
            ),
            Self::OidcClientResourceIdentityProviderMismatch {
                client,
                resource,
                client_identity_provider,
                resource_identity_provider,
            } => write!(
                f,
                "OIDC client `{client}` uses identity provider `{client_identity_provider}` but protected resource `{resource}` uses `{resource_identity_provider}`"
            ),
            Self::UnknownOidcClientSecret { client, secret } => write!(
                f,
                "OIDC client `{client}` references unknown secret `{secret}`"
            ),
            Self::OidcClientSecretPurposeMismatch {
                client,
                secret,
                purpose,
            } => write!(
                f,
                "OIDC client `{client}` references secret `{secret}` with invalid purpose `{purpose:?}`"
            ),
            Self::OidcClientMissingOpenIdScope(client) => {
                write!(f, "OIDC client `{client}` must request the `openid` scope")
            }
            Self::MissingOidcClientForProfile {
                profile,
                identity_provider,
                authorization_server,
            } => write!(
                f,
                "gateway profile `{profile}` advertises OIDC auth without an OIDC client for identity provider `{identity_provider}` and resource authorization server `{authorization_server}`"
            ),
        }
    }
}

impl std::error::Error for GatewayControlPlaneError {}
