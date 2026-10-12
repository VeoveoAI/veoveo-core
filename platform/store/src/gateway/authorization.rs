//! Hosted authorization requests waiting for the identity provider, and the codes the
//! Gateway issues once a request completes. Each is consumed at most once.
use chrono::{DateTime, Utc};
use sqlx::types::Json;
use veoveo_gateway_contract::{
    OAuthAuthorizationCode, OAuthRedirectUri, OAuthStateValue, OidcClientRegistrationId, OidcNonce,
    PkceCodeChallenge, PkceCodeChallengeMethod, PkceCodeVerifier, PrincipalDisplayName,
};
use veoveo_mcp_contract::{GatewayAuthorizationCodeRecord, GatewayAuthorizationRequest, Principal};
use veoveo_types::{GatewayProfileId, OAuthClientId, ScopeName, Vocabulary, WorkContextId};

use super::admitted;
use crate::{PlatformStore, StoreError};

impl PlatformStore {
    pub async fn record_authorization_request(
        &self,
        request: &GatewayAuthorizationRequest,
    ) -> Result<(), StoreError> {
        let scopes = texts(&request.requested_scopes);
        sqlx::query_file!(
            "src/gateway/authorization/create_request.sql",
            request.idp_state.as_str(),
            request.profile.as_str(),
            request.oauth_client_id.as_str(),
            request.work_context.as_str(),
            request.oidc_client.as_str(),
            request.redirect_uri.as_str(),
            request.client_state.as_ref().map(OAuthStateValue::as_str),
            &scopes,
            request.code_challenge.as_str(),
            request.code_challenge_method.as_str(),
            request.idp_code_verifier.as_str(),
            request.idp_code_challenge.as_str(),
            request.idp_code_challenge_method.as_str(),
            request.nonce.as_str(),
            request.created_at,
            request.expires_at,
        )
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Remove and return the unexpired request with this identity-provider state.
    /// Concurrent callers receive it at most once.
    pub async fn consume_authorization_request(
        &self,
        idp_state: &OAuthStateValue,
        now: DateTime<Utc>,
    ) -> Result<Option<GatewayAuthorizationRequest>, StoreError> {
        let row = sqlx::query_file!(
            "src/gateway/authorization/consume_request.sql",
            idp_state.as_str(),
            now,
        )
        .fetch_optional(self.pool())
        .await?;
        row.map(|row| {
            Ok(GatewayAuthorizationRequest {
                idp_state: admitted(
                    "authorization_request.idp_state",
                    OAuthStateValue::parse(row.idp_state),
                )?,
                profile: admitted(
                    "authorization_request.profile",
                    GatewayProfileId::parse(row.profile),
                )?,
                oauth_client_id: admitted(
                    "authorization_request.oauth_client_id",
                    OAuthClientId::parse(row.oauth_client_id),
                )?,
                work_context: admitted(
                    "authorization_request.work_context",
                    WorkContextId::parse(row.work_context),
                )?,
                oidc_client: admitted(
                    "authorization_request.oidc_client",
                    OidcClientRegistrationId::parse(row.oidc_client),
                )?,
                redirect_uri: admitted(
                    "authorization_request.redirect_uri",
                    OAuthRedirectUri::new(row.redirect_uri),
                )?,
                client_state: row
                    .client_state
                    .map(|state| {
                        admitted(
                            "authorization_request.client_state",
                            OAuthStateValue::parse(state),
                        )
                    })
                    .transpose()?,
                requested_scopes: scopes(
                    "authorization_request.requested_scopes",
                    row.requested_scopes,
                )?,
                code_challenge: admitted(
                    "authorization_request.code_challenge",
                    PkceCodeChallenge::parse(row.code_challenge),
                )?,
                code_challenge_method: admitted(
                    "authorization_request.code_challenge_method",
                    PkceCodeChallengeMethod::parse(&row.code_challenge_method),
                )?,
                idp_code_verifier: admitted(
                    "authorization_request.idp_code_verifier",
                    PkceCodeVerifier::parse(row.idp_code_verifier),
                )?,
                idp_code_challenge: admitted(
                    "authorization_request.idp_code_challenge",
                    PkceCodeChallenge::parse(row.idp_code_challenge),
                )?,
                idp_code_challenge_method: admitted(
                    "authorization_request.idp_code_challenge_method",
                    PkceCodeChallengeMethod::parse(&row.idp_code_challenge_method),
                )?,
                nonce: admitted("authorization_request.nonce", OidcNonce::parse(row.nonce))?,
                created_at: row.created_at,
                expires_at: row.expires_at,
            })
        })
        .transpose()
    }

    pub async fn record_authorization_code(
        &self,
        code: &GatewayAuthorizationCodeRecord,
    ) -> Result<(), StoreError> {
        let scopes = texts(&code.scopes);
        sqlx::query_file!(
            "src/gateway/authorization/create_code.sql",
            code.code.as_str(),
            code.profile.as_str(),
            code.oauth_client_id.as_str(),
            code.work_context.as_str(),
            code.oidc_client.as_str(),
            code.redirect_uri.as_str(),
            code.client_state.as_ref().map(OAuthStateValue::as_str),
            &scopes,
            code.code_challenge.as_str(),
            code.code_challenge_method.as_str(),
            Json(&code.principal) as _,
            code.principal_display_name.as_str(),
            code.issued_at,
            code.expires_at,
            code.consumed_at,
        )
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Mark the unexpired, unredeemed code redeemed at `now` and return it. Concurrent
    /// callers redeem it at most once.
    pub async fn consume_authorization_code(
        &self,
        code: &OAuthAuthorizationCode,
        now: DateTime<Utc>,
    ) -> Result<Option<GatewayAuthorizationCodeRecord>, StoreError> {
        let row = sqlx::query_file!(
            "src/gateway/authorization/consume_code.sql",
            code.as_str(),
            now,
        )
        .fetch_optional(self.pool())
        .await?;
        row.map(|row| {
            Ok(GatewayAuthorizationCodeRecord {
                code: admitted(
                    "authorization_code.code",
                    OAuthAuthorizationCode::parse(row.code),
                )?,
                profile: admitted(
                    "authorization_code.profile",
                    GatewayProfileId::parse(row.profile),
                )?,
                oauth_client_id: admitted(
                    "authorization_code.oauth_client_id",
                    OAuthClientId::parse(row.oauth_client_id),
                )?,
                work_context: admitted(
                    "authorization_code.work_context",
                    WorkContextId::parse(row.work_context),
                )?,
                oidc_client: admitted(
                    "authorization_code.oidc_client",
                    OidcClientRegistrationId::parse(row.oidc_client),
                )?,
                redirect_uri: admitted(
                    "authorization_code.redirect_uri",
                    OAuthRedirectUri::new(row.redirect_uri),
                )?,
                client_state: row
                    .client_state
                    .map(|state| {
                        admitted(
                            "authorization_code.client_state",
                            OAuthStateValue::parse(state),
                        )
                    })
                    .transpose()?,
                scopes: scopes("authorization_code.scopes", row.scopes)?,
                code_challenge: admitted(
                    "authorization_code.code_challenge",
                    PkceCodeChallenge::parse(row.code_challenge),
                )?,
                code_challenge_method: admitted(
                    "authorization_code.code_challenge_method",
                    PkceCodeChallengeMethod::parse(&row.code_challenge_method),
                )?,
                principal: row.principal.0,
                principal_display_name: admitted(
                    "authorization_code.principal_display_name",
                    PrincipalDisplayName::new(row.principal_display_name),
                )?,
                issued_at: row.issued_at,
                expires_at: row.expires_at,
                consumed_at: row.consumed_at,
            })
        })
        .transpose()
    }

    /// Delete the requests and codes that expired at or before `now`.
    pub async fn prune_authorization_records(&self, now: DateTime<Utc>) -> Result<u64, StoreError> {
        let deleted = sqlx::query_file_scalar!("src/gateway/authorization/prune.sql", now)
            .fetch_one(self.pool())
            .await?;
        admitted("authorization record count", u64::try_from(deleted))
    }
}

fn texts(scopes: &std::collections::BTreeSet<ScopeName>) -> Vec<String> {
    scopes.iter().map(ToString::to_string).collect()
}

pub(super) fn scopes(
    column: &'static str,
    stored: Vec<String>,
) -> Result<std::collections::BTreeSet<ScopeName>, StoreError> {
    stored
        .into_iter()
        .map(|scope| admitted(column, ScopeName::parse(scope)))
        .collect()
}
