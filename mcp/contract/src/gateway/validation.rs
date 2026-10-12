use std::collections::{BTreeMap, BTreeSet};

use super::*;

pub(super) struct PolicyValidation<'a> {
    pub profiles: &'a BTreeSet<GatewayProfileId>,
    pub protected_resources: &'a BTreeSet<ProtectedResourceId>,
    pub data_labels: &'a BTreeSet<DataLabelId>,
    pub tenants: &'a BTreeSet<TenantId>,
    pub registry: &'a veoveo_gateway_contract::CatalogRegistry,
}

/// Rules govern the Gateway's own actions. Roles grant registered servers' actions.
pub(super) fn validate_policy_set(
    policy: &PolicySet,
    context: PolicyValidation<'_>,
) -> Result<(), GatewayControlPlaneError> {
    let PolicyValidation {
        profiles,
        protected_resources,
        data_labels,
        tenants,
        registry,
    } = context;
    let mut rules = BTreeSet::new();
    for rule in &policy.rules {
        if !rules.insert(rule.id.clone()) {
            return Err(GatewayControlPlaneError::DuplicatePolicyRule {
                policy: policy.version.clone(),
                rule: rule.id.clone(),
            });
        }
        for profile in &rule.profiles {
            if !profiles.contains(profile) {
                return Err(GatewayControlPlaneError::UnknownPolicyRuleProfile {
                    policy: policy.version.clone(),
                    rule: rule.id.clone(),
                    profile: profile.clone(),
                });
            }
        }
        for resource in &rule.protected_resources {
            if !protected_resources.contains(resource) {
                return Err(
                    GatewayControlPlaneError::UnknownPolicyRuleProtectedResource {
                        policy: policy.version.clone(),
                        rule: rule.id.clone(),
                        resource: resource.clone(),
                    },
                );
            }
        }
        validate_policy_rule_actions(policy, rule, registry)?;
        for label in &rule.required_data_labels {
            if !data_labels.contains(label) {
                return Err(GatewayControlPlaneError::UnknownPolicyRuleDataLabel {
                    policy: policy.version.clone(),
                    rule: rule.id.clone(),
                    label: label.clone(),
                });
            }
        }
        for tenant in &rule.tenant_ids {
            if !tenants.contains(tenant) {
                return Err(GatewayControlPlaneError::UnknownPolicyRuleTenant {
                    policy: policy.version.clone(),
                    rule: rule.id.clone(),
                    tenant: tenant.clone(),
                });
            }
        }
    }
    Ok(())
}

fn validate_policy_rule_actions(
    policy: &PolicySet,
    rule: &PolicyRule,
    registry: &veoveo_gateway_contract::CatalogRegistry,
) -> Result<(), GatewayControlPlaneError> {
    use veoveo_gateway_contract::{RuleSelector, SelectorRequirement};
    use veoveo_types::Vocabulary;
    for action in &rule.actions {
        registry
            .actions()
            .resolve(action)
            .map_err(GatewayControlPlaneError::CatalogAdmission)?;
        if let Some(kernel) = GatewayAction::from_wire(action.as_str()) {
            if kernel.targets_registered_server() {
                return Err(GatewayControlPlaneError::PolicyRuleServerAction {
                    policy: policy.version.clone(),
                    rule: rule.id.clone(),
                    action: action.clone(),
                });
            }
            continue;
        }
        let descriptor = registry.descriptor(action).ok_or_else(|| {
            GatewayControlPlaneError::CatalogAdmission(veoveo_types::ExtensionError::new(
                "owner action descriptor is unbound",
            ))
        })?;
        for (selector, empty) in [
            (RuleSelector::Profiles, rule.profiles.is_empty()),
            (
                RuleSelector::ProtectedResources,
                rule.protected_resources.is_empty(),
            ),
        ] {
            match descriptor.selectors[&selector] {
                SelectorRequirement::Required if empty => {}
                SelectorRequirement::Forbidden if !empty => {}
                _ => continue,
            }
            return Err(GatewayControlPlaneError::PolicyRuleUnsupportedAction {
                policy: policy.version.clone(),
                rule: rule.id.clone(),
                action: action.clone(),
            });
        }
    }
    Ok(())
}

pub(super) fn validate_profile_auth_modes(
    profile: &GatewayProfile,
    identity_provider: &IdentityProvider,
    authorization_server: &ResourceAuthorizationServer,
) -> Result<(), GatewayControlPlaneError> {
    if profile.auth_modes.is_empty() {
        return Err(GatewayControlPlaneError::MissingAuthModes {
            profile: profile.id.clone(),
        });
    }
    for auth_mode in &profile.auth_modes {
        match auth_mode {
            AuthMode::OidcAuthorizationCodePkce => {
                require_authorization_server_endpoint(
                    profile,
                    authorization_server,
                    AuthorizationServerEndpoint::Authorization,
                    authorization_server.authorization_endpoint.is_some(),
                )?;
                require_identity_provider_endpoint(
                    profile,
                    identity_provider,
                    IdentityProviderEndpoint::Token,
                    identity_provider.token_endpoint.is_some(),
                )?;
            }
            AuthMode::EnterpriseManagedAuthorization => {
                require_identity_provider_endpoint(
                    profile,
                    identity_provider,
                    IdentityProviderEndpoint::EnterpriseManagedAuthorization,
                    identity_provider
                        .enterprise_managed_authorization_endpoint
                        .is_some(),
                )?;
            }
            AuthMode::OAuthClientCredentials => {}
        }
    }
    Ok(())
}

fn require_identity_provider_endpoint(
    profile: &GatewayProfile,
    identity_provider: &IdentityProvider,
    endpoint: IdentityProviderEndpoint,
    present: bool,
) -> Result<(), GatewayControlPlaneError> {
    if present {
        Ok(())
    } else {
        Err(GatewayControlPlaneError::MissingIdentityProviderEndpoint {
            profile: profile.id.clone(),
            identity_provider: identity_provider.id.clone(),
            endpoint,
        })
    }
}

fn require_authorization_server_endpoint(
    profile: &GatewayProfile,
    authorization_server: &ResourceAuthorizationServer,
    endpoint: AuthorizationServerEndpoint,
    present: bool,
) -> Result<(), GatewayControlPlaneError> {
    if present {
        Ok(())
    } else {
        Err(
            GatewayControlPlaneError::MissingAuthorizationServerEndpoint {
                profile: profile.id.clone(),
                authorization_server: authorization_server.id.clone(),
                endpoint,
            },
        )
    }
}

pub(super) fn validate_oauth_client_registration(
    client: &OAuthClientRegistration,
    authorization_servers: &BTreeMap<AuthorizationServerId, &ResourceAuthorizationServer>,
    profiles: &BTreeMap<GatewayProfileId, &GatewayProfile>,
    contributed_resources: &BTreeMap<
        ProtectedResourceName,
        &veoveo_gateway_contract::ProtectedResourceDescriptor,
    >,
    policies: &BTreeMap<PolicyVersion, &PolicySet>,
    secrets: &BTreeMap<SecretReferenceId, &SecretReference>,
) -> Result<(), GatewayControlPlaneError> {
    if !authorization_servers.contains_key(&client.authorization_server) {
        return Err(
            GatewayControlPlaneError::UnknownOAuthClientAuthorizationServer {
                client: client.id.clone(),
                authorization_server: client.authorization_server.clone(),
            },
        );
    }
    if client.allowed_resources.is_empty() {
        return Err(
            GatewayControlPlaneError::OAuthClientWithoutAllowedResources(client.id.clone()),
        );
    }
    if client.grant_types.is_empty() {
        return Err(GatewayControlPlaneError::OAuthClientWithoutGrantTypes(
            client.id.clone(),
        ));
    }
    if client.auth_methods.is_empty() {
        return Err(GatewayControlPlaneError::OAuthClientWithoutAuthMethods(
            client.id.clone(),
        ));
    }
    validate_oauth_client_surface(client)?;

    for resource_id in &client.allowed_resources {
        let profile = profiles
            .values()
            .copied()
            .find(|profile| &profile.protected_resource == resource_id);
        let contributed = contributed_resources
            .values()
            .copied()
            .find(|resource| &resource.resource == resource_id);
        let (resource_authorization_server, required_scopes) = if let Some(profile) = profile {
            let mut required_scopes = profile
                .required_scopes
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            if let Some(policy) = policies.get(&profile.policy_version) {
                for rule in &policy.rules {
                    if (rule.protected_resources.is_empty()
                        && (rule.profiles.is_empty() || rule.profiles.contains(&profile.id)))
                        || rule
                            .protected_resources
                            .contains(&profile.protected_resource)
                    {
                        required_scopes.extend(rule.required_scopes.iter().cloned());
                    }
                }
            }
            (&profile.authorization_server, required_scopes)
        } else if let Some(resource) = contributed {
            let mut required_scopes = resource.required_scopes.clone();
            if let Some(policy) = policies.get(&resource.policy_version) {
                for rule in &policy.rules {
                    if rule.protected_resources.contains(&resource.resource) {
                        required_scopes.extend(rule.required_scopes.iter().cloned());
                    }
                }
            }
            (&resource.authorization_server, required_scopes)
        } else {
            return Err(GatewayControlPlaneError::UnknownOAuthClientResource {
                client: client.id.clone(),
                resource: resource_id.clone(),
            });
        };
        if resource_authorization_server != &client.authorization_server {
            return Err(
                GatewayControlPlaneError::OAuthClientResourceAuthorizationServerMismatch {
                    client: client.id.clone(),
                    resource: resource_id.clone(),
                    client_authorization_server: client.authorization_server.clone(),
                    resource_authorization_server: resource_authorization_server.clone(),
                },
            );
        }
        for scope in required_scopes {
            if !client.allowed_scopes.contains(&scope) {
                return Err(GatewayControlPlaneError::OAuthClientMissingAllowedScope {
                    client: client.id.clone(),
                    resource: resource_id.clone(),
                    scope,
                });
            }
        }
    }

    if client
        .grant_types
        .contains(&OAuthGrantType::AuthorizationCodePkce)
        && client.redirect_uris.is_empty()
    {
        return Err(GatewayControlPlaneError::OAuthClientMissingRedirectUris {
            client: client.id.clone(),
            grant_type: OAuthGrantType::AuthorizationCodePkce,
        });
    }
    if client
        .grant_types
        .contains(&OAuthGrantType::EnterpriseManagedAuthorization)
        && client.redirect_uris.is_empty()
    {
        return Err(GatewayControlPlaneError::OAuthClientMissingRedirectUris {
            client: client.id.clone(),
            grant_type: OAuthGrantType::EnterpriseManagedAuthorization,
        });
    }
    if client
        .grant_types
        .contains(&OAuthGrantType::ClientCredentials)
        && client.auth_methods.contains(&OAuthClientAuthMethod::None)
    {
        return Err(
            GatewayControlPlaneError::OAuthClientPublicClientCredentials(client.id.clone()),
        );
    }
    validate_oauth_client_auth_configuration(client)?;

    if client
        .auth_methods
        .iter()
        .any(OAuthClientAuthMethod::requires_secret)
    {
        let Some(secret_id) = &client.credential_secret else {
            let auth_method = client
                .auth_methods
                .iter()
                .copied()
                .find(OAuthClientAuthMethod::requires_secret)
                .expect("requires_secret matched");
            return Err(
                GatewayControlPlaneError::OAuthClientMissingCredentialSecret {
                    client: client.id.clone(),
                    auth_method,
                },
            );
        };
        let Some(secret) = secrets.get(secret_id) else {
            return Err(GatewayControlPlaneError::UnknownOAuthClientSecret {
                client: client.id.clone(),
                secret: secret_id.clone(),
            });
        };
        if secret.purpose != SecretPurpose::OAuthClientSecret
            && secret.purpose != SecretPurpose::TokenExchangeCredential
        {
            return Err(GatewayControlPlaneError::OAuthClientSecretPurposeMismatch {
                client: client.id.clone(),
                secret: secret_id.clone(),
                purpose: secret.purpose,
            });
        }
    }

    if client
        .auth_methods
        .iter()
        .any(OAuthClientAuthMethod::requires_jwks)
        && client.jwks.is_none()
    {
        let auth_method = client
            .auth_methods
            .iter()
            .copied()
            .find(OAuthClientAuthMethod::requires_jwks)
            .expect("requires_jwks matched");
        return Err(GatewayControlPlaneError::OAuthClientMissingJwks {
            client: client.id.clone(),
            auth_method,
        });
    }

    Ok(())
}

fn validate_oauth_client_surface(
    client: &OAuthClientRegistration,
) -> Result<(), GatewayControlPlaneError> {
    if client.client_surface == OAuthClientSurface::FullMcp && client.direct_task_call_adapter {
        return Err(
            GatewayControlPlaneError::OAuthClientFullMcpWithCompatibility {
                client: client.id.clone(),
            },
        );
    }
    Ok(())
}

fn validate_oauth_client_auth_configuration(
    client: &OAuthClientRegistration,
) -> Result<(), GatewayControlPlaneError> {
    let browser_grants = client
        .grant_types
        .iter()
        .any(|grant| matches!(grant, OAuthGrantType::AuthorizationCodePkce));
    let refresh_grants = client.grant_types.contains(&OAuthGrantType::RefreshToken);
    let enterprise_managed_grants = client
        .grant_types
        .iter()
        .any(|grant| matches!(grant, OAuthGrantType::EnterpriseManagedAuthorization));
    let client_credentials_grants = client
        .grant_types
        .contains(&OAuthGrantType::ClientCredentials);

    let expected = if client_credentials_grants {
        BTreeSet::from([OAuthClientAuthMethod::PrivateKeyJwt])
    } else if browser_grants || enterprise_managed_grants {
        BTreeSet::from([OAuthClientAuthMethod::None])
    } else {
        BTreeSet::new()
    };

    if expected == client.auth_methods
        && (!refresh_grants || browser_grants)
        && !(client_credentials_grants && (browser_grants || enterprise_managed_grants))
    {
        return Ok(());
    }

    Err(
        GatewayControlPlaneError::OAuthClientUnsupportedAuthConfiguration {
            client: client.id.clone(),
            grant_types: client.grant_types.clone(),
            auth_methods: client.auth_methods.clone(),
        },
    )
}

pub(super) fn validate_oidc_client_registration(
    client: &IdentityProviderOidcClientRegistration,
    identity_providers: &BTreeMap<IdentityProviderId, &IdentityProvider>,
    authorization_servers: &BTreeMap<AuthorizationServerId, &ResourceAuthorizationServer>,
    profiles: &BTreeMap<GatewayProfileId, &GatewayProfile>,
    secrets: &BTreeMap<SecretReferenceId, &SecretReference>,
) -> Result<(), GatewayControlPlaneError> {
    if !identity_providers.contains_key(&client.identity_provider) {
        return Err(
            GatewayControlPlaneError::UnknownOidcClientIdentityProvider {
                client: client.id.clone(),
                identity_provider: client.identity_provider.clone(),
            },
        );
    }
    let Some(authorization_server) = authorization_servers.get(&client.authorization_server) else {
        return Err(
            GatewayControlPlaneError::UnknownOidcClientAuthorizationServer {
                client: client.id.clone(),
                authorization_server: client.authorization_server.clone(),
            },
        );
    };
    if authorization_server.identity_provider.as_ref() != Some(&client.identity_provider) {
        return Err(
            GatewayControlPlaneError::OidcClientAuthorizationServerIdentityProviderMismatch {
                client: client.id.clone(),
                identity_provider: client.identity_provider.clone(),
                authorization_server: client.authorization_server.clone(),
                authorization_server_identity_provider: authorization_server
                    .identity_provider
                    .clone(),
            },
        );
    }
    if client.allowed_resources.is_empty() {
        return Err(GatewayControlPlaneError::OidcClientWithoutAllowedResources(
            client.id.clone(),
        ));
    }
    for resource_id in &client.allowed_resources {
        let Some(profile) = profiles
            .values()
            .copied()
            .find(|profile| &profile.protected_resource == resource_id)
        else {
            return Err(GatewayControlPlaneError::UnknownOidcClientResource {
                client: client.id.clone(),
                resource: resource_id.clone(),
            });
        };
        if profile.authorization_server != client.authorization_server {
            return Err(
                GatewayControlPlaneError::OidcClientResourceAuthorizationServerMismatch {
                    client: client.id.clone(),
                    resource: resource_id.clone(),
                    client_authorization_server: client.authorization_server.clone(),
                    resource_authorization_server: profile.authorization_server.clone(),
                },
            );
        }
        if profile.identity_provider != client.identity_provider {
            return Err(
                GatewayControlPlaneError::OidcClientResourceIdentityProviderMismatch {
                    client: client.id.clone(),
                    resource: resource_id.clone(),
                    client_identity_provider: client.identity_provider.clone(),
                    resource_identity_provider: profile.identity_provider.clone(),
                },
            );
        }
    }
    let Some(secret) = secrets.get(&client.credential_secret) else {
        return Err(GatewayControlPlaneError::UnknownOidcClientSecret {
            client: client.id.clone(),
            secret: client.credential_secret.clone(),
        });
    };
    if secret.purpose != SecretPurpose::OAuthClientSecret {
        return Err(GatewayControlPlaneError::OidcClientSecretPurposeMismatch {
            client: client.id.clone(),
            secret: client.credential_secret.clone(),
            purpose: secret.purpose,
        });
    }
    if !client
        .scopes
        .contains(&ScopeName::parse("openid").expect("valid literal"))
    {
        return Err(GatewayControlPlaneError::OidcClientMissingOpenIdScope(
            client.id.clone(),
        ));
    }
    Ok(())
}
