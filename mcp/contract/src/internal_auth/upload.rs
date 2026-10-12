//! Upload-only assertion binds the gateway decision to its checked configuration.

use super::*;
use veoveo_artifact_contract::{ARTIFACT_UPLOAD_AUDIENCE, ArtifactUploadAuthority};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedArtifactUploadIdentity {
    pub identity: GatewayInternalIdentity,
    pub authorization: ArtifactUploadAuthority,
}

/// Private claim binding an upload token to the Gateway's checked configuration.
pub const ARTIFACT_UPLOAD_CLAIM: &str = "ai.veoveo/artifact-upload";

#[derive(Serialize, Deserialize)]
struct UploadClaims {
    #[serde(flatten)]
    identity: AccessTokenClaims,
    #[serde(rename = "ai.veoveo/artifact-upload")]
    upload_authorization: ArtifactUploadAuthority,
}

impl GatewayInternalTokenIssuer {
    pub fn issue_artifact_upload(
        &self,
        profile: GatewayProfileId,
        actor: Principal,
        authority: InvocationAuthority,
        request_context: GatewayRequestContext,
        authorization: ArtifactUploadAuthority,
        expires_at: DateTime<Utc>,
    ) -> Result<String, InternalTokenError> {
        let identity = self.create_identity(
            profile,
            ServerSlug::parse(ARTIFACT_UPLOAD_AUDIENCE).map_err(InternalTokenError::Identifier)?,
            actor,
            authority,
            BTreeSet::new(),
            Some(request_context),
            expires_at,
        )?;
        self.sign(&UploadClaims {
            identity: AccessTokenClaims::from_identity(&identity)?,
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
        let actual = claims.identity.standard.audience()?;
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
