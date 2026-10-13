//! Rotating refresh-token families. Every change to a family's tokens holds the family's
//! row lock, so concurrent presentations of one token serialize across replicas: the
//! first rotates it, a presentation within the successor's delivery window receives the
//! same successor, and a later one revokes the family as a replay.
use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use secrecy::SecretString;
use sqlx::{PgConnection, types::Json};
use uuid::Uuid;
use veoveo_gateway_contract::{AuthorizationServerId, PrincipalDisplayName};
use veoveo_mcp_contract::Principal;
use veoveo_types::{
    GatewayProfileId, GatewayRefreshFamilyId, OAuthClientId, ScopeName, Sha256Digest, WorkContextId,
};

use super::{TransactionAppend, admitted, authorization::scopes};
use crate::{MaintenanceBatch, PlatformStore, Pruned, StoreError};

/// Why a family stopped issuing access.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, veoveo_types::Vocabulary)]
#[vocabulary(postgres)]
pub enum RefreshRevocation {
    /// The client revoked its refresh token (RFC 7009).
    #[vocabulary(rename = "client_revocation")]
    ClientRevocation,
    /// A consumed token was presented after its successor's delivery window.
    #[vocabulary(rename = "token_replay")]
    TokenReplay,
}

/// The client a family belongs to. A token presented by another client, profile or
/// authorization server is not this family's.
#[derive(Clone, Copy, Debug)]
pub struct RefreshBinding<'a> {
    pub authorization_server: &'a AuthorizationServerId,
    pub profile: &'a GatewayProfileId,
    pub oauth_client_id: &'a OAuthClientId,
}

/// A family and the principal it was issued to.
#[derive(Clone, Debug, PartialEq)]
pub struct RefreshFamily {
    pub id: GatewayRefreshFamilyId,
    pub authorization_server: AuthorizationServerId,
    pub profile: GatewayProfileId,
    pub oauth_client_id: OAuthClientId,
    pub work_context: WorkContextId,
    pub principal: RefreshPrincipal,
    pub scopes: BTreeSet<ScopeName>,
    pub current_generation: u64,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revocation: Option<RefreshRevocation>,
}

impl RefreshFamily {
    /// Whether a token of this family presented by `binding` belongs to it.
    pub fn bound_to(&self, binding: RefreshBinding<'_>) -> bool {
        &self.authorization_server == binding.authorization_server
            && &self.profile == binding.profile
            && &self.oauth_client_id == binding.oauth_client_id
    }
}

/// The principal a family's access tokens name, with its display name.
#[derive(Clone, Debug, PartialEq)]
pub struct RefreshPrincipal {
    pub principal: Principal,
    pub display_name: PrincipalDisplayName,
}

/// A family before its first token.
#[derive(Clone, Debug)]
pub struct NewRefreshFamily {
    pub id: GatewayRefreshFamilyId,
    pub authorization_server: AuthorizationServerId,
    pub profile: GatewayProfileId,
    pub oauth_client_id: OAuthClientId,
    pub work_context: WorkContextId,
    pub principal: RefreshPrincipal,
    pub scopes: BTreeSet<ScopeName>,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// The encrypted successor a concurrent presentation of its predecessor receives, and
/// when that window ends.
#[derive(Clone, Debug)]
pub struct RefreshDeliveryEnvelope {
    pub envelope: SecretString,
    pub expires_at: DateTime<Utc>,
}

/// One generation of a family. The store holds the token's digest, never the token.
#[derive(Clone, Debug)]
pub struct RefreshToken {
    pub id: Uuid,
    pub family: GatewayRefreshFamilyId,
    pub digest: Sha256Digest,
    pub generation: u64,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
    pub replacement: Option<Uuid>,
    pub replay_detected_at: Option<DateTime<Utc>>,
    pub delivery: Option<RefreshDeliveryEnvelope>,
}

/// A token to store. Its family is the one it is stored for.
#[derive(Clone, Debug)]
pub struct NewRefreshToken {
    pub id: Uuid,
    pub digest: Sha256Digest,
    pub generation: u64,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub delivery: Option<RefreshDeliveryEnvelope>,
}

/// What presenting a refresh token for rotation did.
#[derive(Clone, Debug)]
pub enum RefreshRotation {
    /// The token was current: it is consumed and `replacement` is the family's new
    /// generation.
    Rotated {
        family: RefreshFamily,
        replacement: RefreshToken,
    },
    /// The token was consumed moments ago: `replacement` is the successor that
    /// rotation delivered, still inside its delivery window.
    Redelivered {
        family: RefreshFamily,
        replacement: RefreshToken,
    },
    /// The token was consumed earlier: the family is revoked.
    ReplayDetected(RefreshFamily),
    /// The token is unknown, expired, revoked or not the family's current generation.
    Invalid,
}

/// What one retention pass changed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RefreshRetention {
    pub delivery_envelopes_cleared: Pruned,
    pub tokens_deleted: Pruned,
    pub families_deleted: Pruned,
}

struct FamilyRow {
    id: Uuid,
    authorization_server: String,
    profile: String,
    oauth_client_id: String,
    work_context: String,
    principal: Json<Principal>,
    principal_display_name: String,
    scopes: Vec<String>,
    current_generation: i64,
    issued_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
    revocation_reason: Option<RefreshRevocation>,
}

impl TryFrom<FamilyRow> for RefreshFamily {
    type Error = StoreError;

    fn try_from(row: FamilyRow) -> Result<Self, StoreError> {
        Ok(Self {
            id: family_id(row.id)?,
            authorization_server: admitted(
                "refresh_family.authorization_server",
                AuthorizationServerId::parse(row.authorization_server),
            )?,
            profile: admitted(
                "refresh_family.profile",
                GatewayProfileId::parse(row.profile),
            )?,
            oauth_client_id: admitted(
                "refresh_family.oauth_client_id",
                OAuthClientId::parse(row.oauth_client_id),
            )?,
            work_context: admitted(
                "refresh_family.work_context",
                WorkContextId::parse(row.work_context),
            )?,
            principal: RefreshPrincipal {
                principal: row.principal.0,
                display_name: admitted(
                    "refresh_family.principal_display_name",
                    PrincipalDisplayName::new(row.principal_display_name),
                )?,
            },
            scopes: scopes("refresh_family.scopes", row.scopes)?,
            current_generation: admitted(
                "refresh_family.current_generation",
                u64::try_from(row.current_generation),
            )?,
            issued_at: row.issued_at,
            expires_at: row.expires_at,
            revoked_at: row.revoked_at,
            revocation: row.revocation_reason,
        })
    }
}

struct TokenRow {
    id: Uuid,
    family: Uuid,
    token_digest: String,
    generation: i64,
    issued_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    consumed_at: Option<DateTime<Utc>>,
    replacement: Option<Uuid>,
    replay_detected_at: Option<DateTime<Utc>>,
    delivery_envelope: Option<String>,
    delivery_expires_at: Option<DateTime<Utc>>,
}

impl TryFrom<TokenRow> for RefreshToken {
    type Error = StoreError;

    fn try_from(row: TokenRow) -> Result<Self, StoreError> {
        let delivery = match (row.delivery_envelope, row.delivery_expires_at) {
            (Some(envelope), Some(expires_at)) => Some(RefreshDeliveryEnvelope {
                envelope: SecretString::from(envelope),
                expires_at,
            }),
            (None, None) => None,
            _ => {
                return Err(StoreError::InvalidStoredValue {
                    column: "refresh_token.delivery_envelope",
                });
            }
        };
        Ok(Self {
            id: row.id,
            family: family_id(row.family)?,
            digest: admitted(
                "refresh_token.token_digest",
                Sha256Digest::parse(row.token_digest),
            )?,
            generation: admitted("refresh_token.generation", u64::try_from(row.generation))?,
            issued_at: row.issued_at,
            expires_at: row.expires_at,
            consumed_at: row.consumed_at,
            replacement: row.replacement,
            replay_detected_at: row.replay_detected_at,
            delivery,
        })
    }
}

fn family_id(id: Uuid) -> Result<GatewayRefreshFamilyId, StoreError> {
    admitted(
        "refresh family id",
        GatewayRefreshFamilyId::parse(id.to_string()),
    )
}

fn generation(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::InvalidRefreshTransition {
        reason: "the generation exceeds the stored range",
    })
}

impl PlatformStore {
    /// Store a new family with its generation-0 token in one transaction.
    pub async fn create_refresh_family(
        &self,
        family: &NewRefreshFamily,
        token: &NewRefreshToken,
    ) -> Result<(), StoreError> {
        if token.generation != 0
            || token.expires_at != family.expires_at
            || token.issued_at >= token.expires_at
            || token.delivery.is_some()
        {
            return Err(StoreError::InvalidRefreshTransition {
                reason: "a family starts with an undelivered generation-0 token of its lifetime",
            });
        }
        let principal = &family.principal.principal;
        let scopes = family
            .scopes
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let mut transaction = self.pool().begin().await?;
        sqlx::query_file!(
            "src/gateway/refresh/create_family.sql",
            family.id.as_uuid(),
            family.authorization_server.as_str(),
            family.profile.as_str(),
            family.oauth_client_id.as_str(),
            family.work_context.as_str(),
            principal.id.as_str(),
            principal.tenant.as_ref().map(|tenant| tenant.as_str()),
            &scopes,
            Json(principal) as _,
            family.principal.display_name.as_str(),
            family.issued_at,
            family.expires_at,
        )
        .execute(&mut *transaction)
        .await?;
        insert_token(&mut transaction, &family.id, token).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// The token with this digest and its family, as currently stored.
    pub async fn refresh_grant(
        &self,
        digest: &Sha256Digest,
    ) -> Result<Option<(RefreshToken, RefreshFamily)>, StoreError> {
        let mut connection = self.pool().acquire().await?;
        let Some(token) = token_by_digest(&mut connection, digest).await? else {
            return Ok(None);
        };
        let family = sqlx::query_file_as!(
            FamilyRow,
            "src/gateway/refresh/family.sql",
            token.family.as_uuid(),
        )
        .fetch_optional(&mut *connection)
        .await?;
        family
            .map(|family| Ok((token, RefreshFamily::try_from(family)?)))
            .transpose()
    }

    /// The family with this identifier.
    pub async fn refresh_family(
        &self,
        id: &GatewayRefreshFamilyId,
    ) -> Result<Option<RefreshFamily>, StoreError> {
        sqlx::query_file_as!(FamilyRow, "src/gateway/refresh/family.sql", id.as_uuid())
            .fetch_optional(self.pool())
            .await?
            .map(RefreshFamily::try_from)
            .transpose()
    }

    /// Revoke the family of a token its own client presents (RFC 7009). Revoking a
    /// revoked family succeeds again; a token that is unknown, expired or another
    /// client's revokes nothing.
    pub async fn revoke_refresh_family(
        &self,
        digest: &Sha256Digest,
        binding: RefreshBinding<'_>,
        now: DateTime<Utc>,
    ) -> Result<Option<RefreshFamily>, StoreError> {
        let mut transaction = self.pool().begin().await?;
        let Some(mut family) = lock_family(&mut transaction, digest).await? else {
            return Ok(None);
        };
        let Some(token) = token_by_digest(&mut transaction, digest).await? else {
            return Ok(None);
        };
        if !family.bound_to(binding) || token.expires_at <= now || family.expires_at <= now {
            return Ok(None);
        }
        if family.revoked_at.is_none() {
            revoke(
                &mut transaction,
                &mut family,
                now,
                RefreshRevocation::ClientRevocation,
            )
            .await?;
            transaction.commit().await?;
        }
        Ok(Some(family))
    }

    /// Rotate the token with digest `current` to `replacement`, the family's next
    /// generation, and append `rotated` in the same transaction. A token consumed within
    /// its successor's delivery window redelivers that successor and appends
    /// `redelivered` instead; a token consumed earlier revokes its family.
    pub async fn rotate_refresh_token(
        &self,
        current: &Sha256Digest,
        replacement: &NewRefreshToken,
        now: DateTime<Utc>,
        rotated: &impl TransactionAppend,
        redelivered: &impl TransactionAppend,
    ) -> Result<RefreshRotation, StoreError> {
        let mut transaction = self.pool().begin().await?;
        let Some(mut family) = lock_family(&mut transaction, current).await? else {
            return Ok(RefreshRotation::Invalid);
        };
        let Some(token) = token_by_digest(&mut transaction, current).await? else {
            return Ok(RefreshRotation::Invalid);
        };
        if Some(replacement.generation) != token.generation.checked_add(1)
            || replacement.expires_at != family.expires_at
            || replacement.issued_at >= replacement.expires_at
            || replacement.delivery.is_none()
        {
            return Err(StoreError::InvalidRefreshTransition {
                reason: "the replacement is not the delivered next generation of the family",
            });
        }
        if token.consumed_at.is_some() || token.replay_detected_at.is_some() {
            let successor = match token.replacement {
                Some(id) => sqlx::query_file_as!(TokenRow, "src/gateway/refresh/token.sql", id)
                    .fetch_optional(&mut *transaction)
                    .await?
                    .map(RefreshToken::try_from)
                    .transpose()?,
                None => None,
            };
            if let Some(successor) =
                successor.filter(|successor| deliverable(&family, &token, successor, now))
            {
                redelivered.append(&mut transaction).await?;
                transaction.commit().await?;
                return Ok(RefreshRotation::Redelivered {
                    family,
                    replacement: successor,
                });
            }
            revoke(
                &mut transaction,
                &mut family,
                now,
                RefreshRevocation::TokenReplay,
            )
            .await?;
            sqlx::query_file!("src/gateway/refresh/mark_replay.sql", token.id, now)
                .execute(&mut *transaction)
                .await?;
            transaction.commit().await?;
            return Ok(RefreshRotation::ReplayDetected(family));
        }
        if token.expires_at <= now
            || family.expires_at <= now
            || family.revoked_at.is_some()
            || token.generation != family.current_generation
        {
            return Ok(RefreshRotation::Invalid);
        }
        insert_token(&mut transaction, &family.id, replacement).await?;
        sqlx::query_file!(
            "src/gateway/refresh/consume_token.sql",
            token.id,
            now,
            replacement.id,
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query_file!(
            "src/gateway/refresh/advance_family.sql",
            family.id.as_uuid(),
            generation(replacement.generation)?,
        )
        .execute(&mut *transaction)
        .await?;
        rotated.append(&mut transaction).await?;
        transaction.commit().await?;
        family.current_generation = replacement.generation;
        Ok(RefreshRotation::Rotated {
            family: family.clone(),
            replacement: RefreshToken {
                id: replacement.id,
                family: family.id,
                digest: replacement.digest.clone(),
                generation: replacement.generation,
                issued_at: replacement.issued_at,
                expires_at: replacement.expires_at,
                consumed_at: None,
                replacement: None,
                replay_detected_at: None,
                delivery: replacement.delivery.clone(),
            },
        })
    }

    /// Clear delivery envelopes whose window ended at or before `now`, then delete the
    /// tokens and families that expired, each in batches of their own.
    pub async fn prune_refresh_state(
        &self,
        now: DateTime<Utc>,
        batch: MaintenanceBatch,
    ) -> Result<RefreshRetention, StoreError> {
        let delivery_envelopes_cleared = self.clear_refresh_delivery_envelopes(now, batch).await?;
        let tokens_deleted = Pruned::in_batches(batch, |limit| async move {
            Ok::<_, StoreError>(
                sqlx::query_file!("src/gateway/refresh/prune_tokens.sql", now, limit)
                    .execute(self.pool())
                    .await?
                    .rows_affected(),
            )
        })
        .await?;
        let families_deleted = Pruned::in_batches(batch, |limit| async move {
            Ok::<_, StoreError>(
                sqlx::query_file!("src/gateway/refresh/prune_families.sql", now, limit)
                    .execute(self.pool())
                    .await?
                    .rows_affected(),
            )
        })
        .await?;
        Ok(RefreshRetention {
            delivery_envelopes_cleared,
            tokens_deleted,
            families_deleted,
        })
    }

    /// Clear delivery envelopes whose window ended at or before `now`, in batches.
    pub async fn clear_refresh_delivery_envelopes(
        &self,
        now: DateTime<Utc>,
        batch: MaintenanceBatch,
    ) -> Result<Pruned, StoreError> {
        Pruned::in_batches(batch, |limit| async move {
            Ok(
                sqlx::query_file!("src/gateway/refresh/clear_envelopes.sql", now, limit)
                    .execute(self.pool())
                    .await?
                    .rows_affected(),
            )
        })
        .await
    }
}

/// Whether `successor` is the delivered successor of the consumed `token`, still
/// inside its delivery window.
fn deliverable(
    family: &RefreshFamily,
    token: &RefreshToken,
    successor: &RefreshToken,
    now: DateTime<Utc>,
) -> bool {
    token.consumed_at.is_some()
        && token.replay_detected_at.is_none()
        && family.revoked_at.is_none()
        && family.expires_at > now
        && successor.family == family.id
        && Some(successor.generation) == token.generation.checked_add(1)
        && successor.generation == family.current_generation
        && successor.expires_at > now
        && successor.consumed_at.is_none()
        && successor.replay_detected_at.is_none()
        && successor
            .delivery
            .as_ref()
            .is_some_and(|delivery| delivery.expires_at > now)
}

async fn lock_family(
    connection: &mut PgConnection,
    digest: &Sha256Digest,
) -> Result<Option<RefreshFamily>, StoreError> {
    sqlx::query_file_as!(
        FamilyRow,
        "src/gateway/refresh/lock_family.sql",
        digest.as_str(),
    )
    .fetch_optional(&mut *connection)
    .await?
    .map(RefreshFamily::try_from)
    .transpose()
}

async fn token_by_digest(
    connection: &mut PgConnection,
    digest: &Sha256Digest,
) -> Result<Option<RefreshToken>, StoreError> {
    sqlx::query_file_as!(
        TokenRow,
        "src/gateway/refresh/token_by_digest.sql",
        digest.as_str(),
    )
    .fetch_optional(&mut *connection)
    .await?
    .map(RefreshToken::try_from)
    .transpose()
}

async fn insert_token(
    connection: &mut PgConnection,
    family: &GatewayRefreshFamilyId,
    token: &NewRefreshToken,
) -> Result<(), StoreError> {
    use secrecy::ExposeSecret;
    sqlx::query_file!(
        "src/gateway/refresh/create_token.sql",
        token.id,
        family.as_uuid(),
        token.digest.as_str(),
        generation(token.generation)?,
        token.issued_at,
        token.expires_at,
        token
            .delivery
            .as_ref()
            .map(|delivery| delivery.envelope.expose_secret()),
        token.delivery.as_ref().map(|delivery| delivery.expires_at),
    )
    .execute(&mut *connection)
    .await?;
    Ok(())
}

async fn revoke(
    connection: &mut PgConnection,
    family: &mut RefreshFamily,
    now: DateTime<Utc>,
    reason: RefreshRevocation,
) -> Result<(), StoreError> {
    let revoked = sqlx::query_file!(
        "src/gateway/refresh/revoke_family.sql",
        family.id.as_uuid(),
        now,
        reason as RefreshRevocation,
    )
    .fetch_one(&mut *connection)
    .await?;
    family.revoked_at = Some(revoked.revoked_at);
    family.revocation = Some(revoked.revocation_reason);
    Ok(())
}
