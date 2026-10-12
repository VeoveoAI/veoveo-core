//! Upload-only assertion binds the gateway decision to its checked configuration.

use super::*;
use veoveo_artifact_contract::{ARTIFACT_UPLOAD_AUDIENCE, ArtifactUploadAuthority};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedArtifactUploadIdentity {
    pub identity: GatewayInternalIdentity,
    pub authorization: ArtifactUploadAuthority,
}

/// Public claim, under a collision-resistant URI name, binding an upload token to the
/// Gateway's checked configuration.
pub const ARTIFACT_UPLOAD_CLAIM: &str = "https://veoveo.ai/claims/artifact-upload";

#[derive(Serialize, Deserialize)]
struct UploadClaims {
    #[serde(flatten)]
    identity: AccessTokenClaims,
    #[serde(rename = "https://veoveo.ai/claims/artifact-upload")]
    upload_authorization: ArtifactUploadAuthority,
}

impl GatewayInternalTokenIssuer {
    /// `audience` is the upload service's resource URL.
    #[allow(clippy::too_many_arguments)]
    pub fn issue_artifact_upload(
        &self,
        profile: GatewayProfileId,
        audience: ResourceIndicator,
        actor: Principal,
        authority: InvocationAuthority,
        request_context: GatewayRequestContext,
        authorization: ArtifactUploadAuthority,
        expires_at: DateTime<Utc>,
    ) -> Result<String, InternalTokenError> {
        let identity = self.create_identity(ForwardedRequest {
            profile,
            server: ServerSlug::parse(ARTIFACT_UPLOAD_AUDIENCE)
                .map_err(InternalTokenError::Identifier)?,
            audience,
            client: request_context.access_token.oauth_client_id.clone(),
            actor,
            authority,
            permissions: BTreeSet::new(),
            request_context: Some(request_context),
            expires_at,
        })?;
        self.sign(&UploadClaims {
            identity: AccessTokenClaims::from_identity(&identity),
            upload_authorization: authorization,
        })
    }
}

impl GatewayInternalTokenVerifier {
    pub fn verify_artifact_upload(
        &self,
        token: &str,
    ) -> Result<VerifiedArtifactUploadIdentity, InternalTokenError> {
        let claims = self.decode_claims::<UploadClaims>(token)?;
        let expected =
            ServerSlug::parse(ARTIFACT_UPLOAD_AUDIENCE).map_err(InternalTokenError::Identifier)?;
        let actual = claims.identity.invocation.server.clone();
        if actual != expected {
            return Err(InternalTokenError::AudienceMismatch { expected, actual });
        }
        let identity = self.identity_from_claims(claims.identity)?;
        identity.audit_context()?;
        Ok(VerifiedArtifactUploadIdentity {
            identity,
            authorization: claims.upload_authorization,
        })
    }
}
