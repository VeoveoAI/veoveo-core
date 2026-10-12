//! The Gateway's state against a private PostgreSQL 18 cluster.
#[allow(dead_code, reason = "each test binary uses part of the shared fixture")]
#[path = "../../../testing/fixtures/store.rs"]
mod store;

use std::collections::BTreeSet;

use chrono::{DateTime, SubsecRound, TimeDelta, Utc};
use futures::StreamExt;
use secrecy::SecretString;
use sqlx::{PgConnection, types::Json};
use store::TestDb;
use uuid::Uuid;
use veoveo_gateway_contract::{
    AuthorizationServerId, JwtId, OAuthAuthorizationCode, OAuthRedirectUri, OAuthStateValue,
    OidcClientRegistrationId, OidcNonce, PkceCodeChallenge, PkceCodeChallengeMethod,
    PkceCodeVerifier, PrincipalDisplayName,
};
use veoveo_mcp_contract::{
    GatewayAuthorizationCodeRecord, GatewayAuthorizationRequest, GatewayControlPlane,
    GatewayControlPlaneRevision, GatewayControlPlaneRevisionId, GatewayControlPlaneRevisionSource,
    GatewayJwtRevocation, Principal,
};
use veoveo_platform_store::{
    ChangeOperation, ChangeSignal, Channel, PrincipalKind, StoreError, TenantId as TenantRow,
    WorkContextId as WorkContextRow, deterministic_principal_id, deterministic_tenant_id,
    deterministic_work_context_id,
    gateway::{
        ControlObject, ControlPublication, NewRefreshFamily, NewRefreshToken, RefreshBinding,
        RefreshDeliveryEnvelope, RefreshPrincipal, RefreshRevocation, RefreshRotation,
        ReplayIdentifier, ReplayKind, TransactionAppend,
    },
};
use veoveo_types::{
    ExtensionName, GatewayProfileId, OAuthClientId, PolicyVersion, PrincipalId, ScopeName,
    Sha256Digest, TenantId, TokenIssuer, WorkContextId,
};

/// Appends nothing, standing in for the audit records a change commits with.
struct Nothing;

impl TransactionAppend for Nothing {
    async fn append(&self, _: &mut PgConnection) -> Result<(), StoreError> {
        Ok(())
    }
}

/// Refuses its records, as an audit record whose identity holds other content does.
struct Refused;

impl TransactionAppend for Refused {
    async fn append(&self, _: &mut PgConnection) -> Result<(), StoreError> {
        Err(StoreError::Append("refused".into()))
    }
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

fn digest(byte: u8) -> Sha256Digest {
    Sha256Digest::from_bytes([byte; 32])
}

fn principal() -> Principal {
    serde_json::from_value(serde_json::json!({
        "id": "https://issuer.example#alice",
        "kind": "user",
        "issuer": "https://issuer.example",
        "subject": "alice",
        "tenant": "acme",
        "scopes": ["operator:use"]
    }))
    .unwrap()
}

#[tokio::test]
async fn replay_identifiers_are_claimed_once_across_pools_until_they_expire() {
    let db = TestDb::new().await;
    let server = AuthorizationServerId::parse("veoveo").unwrap();
    let client = OAuthClientId::parse("console").unwrap();
    let jwt = JwtId::parse("assertion-1").unwrap();
    let start = now();
    let identifier = |kind, seen: DateTime<Utc>| ReplayIdentifier {
        kind,
        authorization_server: &server,
        client_id: &client,
        jwt_id: &jwt,
        expires_at: seen + TimeDelta::minutes(5),
    };
    let claims = futures::future::join_all((0..8).map(|index| {
        let store = if index % 2 == 0 { &db.a } else { &db.b };
        store.claim_replay_id(identifier(ReplayKind::ClientAssertion, start), start)
    }))
    .await;
    assert_eq!(
        claims
            .into_iter()
            .map(Result::unwrap)
            .filter(|claimed| *claimed)
            .count(),
        1
    );
    assert!(
        db.a.claim_replay_id(identifier(ReplayKind::IdJag, start), start)
            .await
            .unwrap(),
        "each kind keeps its own identifiers"
    );
    let later = start + TimeDelta::minutes(6);
    assert!(
        db.b.claim_replay_id(identifier(ReplayKind::ClientAssertion, later), later)
            .await
            .unwrap(),
        "an expired claim passes to a new caller"
    );
    assert!(
        !db.a
            .claim_replay_id(identifier(ReplayKind::ClientAssertion, later), later)
            .await
            .unwrap(),
        "the new claim is live again"
    );
    assert_eq!(
        db.a.prune_replay_ids(ReplayKind::IdJag, later)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.a.prune_replay_ids(ReplayKind::ClientAssertion, later)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn authorization_requests_and_codes_are_consumed_once() {
    let db = TestDb::new().await;
    let start = now();
    let request = GatewayAuthorizationRequest {
        idp_state: OAuthStateValue::parse("state-1").unwrap(),
        profile: GatewayProfileId::parse("operator").unwrap(),
        oauth_client_id: OAuthClientId::parse("console").unwrap(),
        work_context: WorkContextId::parse("mission").unwrap(),
        oidc_client: OidcClientRegistrationId::parse("enterprise").unwrap(),
        redirect_uri: OAuthRedirectUri::new("https://veoveo.example/oauth/callback").unwrap(),
        client_state: None,
        requested_scopes: BTreeSet::from([ScopeName::parse("operator:use").unwrap()]),
        code_challenge: PkceCodeChallenge::parse("A".repeat(43)).unwrap(),
        code_challenge_method: PkceCodeChallengeMethod::S256,
        idp_code_verifier: PkceCodeVerifier::parse("B".repeat(43)).unwrap(),
        idp_code_challenge: PkceCodeChallenge::parse("C".repeat(43)).unwrap(),
        idp_code_challenge_method: PkceCodeChallengeMethod::S256,
        nonce: OidcNonce::parse("nonce-1").unwrap(),
        created_at: start,
        expires_at: start + TimeDelta::minutes(5),
    };
    db.a.record_authorization_request(&request).await.unwrap();
    let (left, right) = tokio::join!(
        db.a.consume_authorization_request(&request.idp_state, start),
        db.b.consume_authorization_request(&request.idp_state, start),
    );
    let consumed = [left.unwrap(), right.unwrap()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(consumed, vec![request.clone()]);

    let code = GatewayAuthorizationCodeRecord {
        code: OAuthAuthorizationCode::parse("D".repeat(43)).unwrap(),
        profile: request.profile.clone(),
        oauth_client_id: request.oauth_client_id.clone(),
        work_context: request.work_context.clone(),
        oidc_client: request.oidc_client.clone(),
        redirect_uri: request.redirect_uri.clone(),
        client_state: Some(OAuthStateValue::parse("client-state").unwrap()),
        scopes: request.requested_scopes.clone(),
        code_challenge: request.code_challenge.clone(),
        code_challenge_method: PkceCodeChallengeMethod::S256,
        principal: principal(),
        principal_display_name: PrincipalDisplayName::new("Alice").unwrap(),
        issued_at: start,
        expires_at: start + TimeDelta::minutes(5),
        consumed_at: None,
    };
    db.a.record_authorization_code(&code).await.unwrap();
    let (left, right) = tokio::join!(
        db.a.consume_authorization_code(&code.code, start),
        db.b.consume_authorization_code(&code.code, start),
    );
    let consumed = [left.unwrap(), right.unwrap()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(
        consumed,
        vec![GatewayAuthorizationCodeRecord {
            consumed_at: Some(start),
            ..code
        }]
    );
    assert_eq!(
        db.a.prune_authorization_records(start + TimeDelta::minutes(6))
            .await
            .unwrap(),
        1,
        "the consumed request is gone and the redeemed code expires"
    );
}

#[tokio::test]
async fn jwt_revocations_apply_until_the_token_expires() {
    let db = TestDb::new().await;
    let start = now();
    let mut revocation = GatewayJwtRevocation {
        profile: GatewayProfileId::parse("operator").unwrap(),
        issuer: TokenIssuer::parse("https://issuer.example").unwrap(),
        jwt_id: JwtId::parse("token-1").unwrap(),
        revoked_at: start,
        expires_at: start + TimeDelta::hours(1),
        reason: None,
    };
    db.a.record_jwt_revocation(&revocation).await.unwrap();
    revocation.reason = Some("operator request".to_owned());
    db.a.record_jwt_revocation(&revocation).await.unwrap();
    let read = |at| {
        db.b.jwt_revocation(
            &revocation.profile,
            &revocation.issuer,
            &revocation.jwt_id,
            at,
        )
    };
    assert_eq!(read(start).await.unwrap(), Some(revocation.clone()));
    assert_eq!(read(start + TimeDelta::hours(2)).await.unwrap(), None);
    assert_eq!(
        db.a.prune_jwt_revocations(start + TimeDelta::hours(2))
            .await
            .unwrap(),
        1
    );
}

struct Family {
    family: NewRefreshFamily,
    binding_server: AuthorizationServerId,
}

impl Family {
    fn new(start: DateTime<Utc>) -> Self {
        let binding_server = AuthorizationServerId::parse("veoveo").unwrap();
        Self {
            family: NewRefreshFamily {
                id: veoveo_types::GatewayRefreshFamilyId::parse(Uuid::now_v7().to_string())
                    .unwrap(),
                authorization_server: binding_server.clone(),
                profile: GatewayProfileId::parse("operator").unwrap(),
                oauth_client_id: OAuthClientId::parse("console").unwrap(),
                work_context: WorkContextId::parse("mission").unwrap(),
                principal: RefreshPrincipal {
                    principal: principal(),
                    display_name: PrincipalDisplayName::new("Alice").unwrap(),
                },
                scopes: BTreeSet::from([ScopeName::parse("operator:use").unwrap()]),
                issued_at: start,
                expires_at: start + TimeDelta::days(7),
            },
            binding_server,
        }
    }

    fn binding(&self) -> RefreshBinding<'_> {
        RefreshBinding {
            authorization_server: &self.binding_server,
            profile: &self.family.profile,
            oauth_client_id: &self.family.oauth_client_id,
        }
    }

    fn token(&self, generation: u64, byte: u8, at: DateTime<Utc>) -> NewRefreshToken {
        NewRefreshToken {
            id: Uuid::now_v7(),
            digest: digest(byte),
            generation,
            issued_at: at,
            expires_at: self.family.expires_at,
            delivery: (generation > 0).then(|| RefreshDeliveryEnvelope {
                envelope: SecretString::from(format!("envelope-{byte}")),
                expires_at: at + TimeDelta::seconds(5),
            }),
        }
    }
}

#[tokio::test]
async fn concurrent_presentations_rotate_once_redeliver_the_successor_and_revoke_on_replay() {
    let db = TestDb::new().await;
    let start = now();
    let family = Family::new(start);
    db.a.create_refresh_family(&family.family, &family.token(0, 1, start))
        .await
        .unwrap();
    let at = start + TimeDelta::seconds(1);
    let current = digest(1);
    let (one, two) = (family.token(1, 2, at), family.token(1, 3, at));
    let (left, right) = tokio::join!(
        db.a.rotate_refresh_token(&current, &one, at, &Nothing, &Nothing),
        db.b.rotate_refresh_token(&current, &two, at, &Nothing, &Nothing),
    );
    let mut rotated = None;
    let mut redelivered = None;
    for outcome in [left.unwrap(), right.unwrap()] {
        match outcome {
            RefreshRotation::Rotated { replacement, .. } => rotated = Some(replacement),
            RefreshRotation::Redelivered { replacement, .. } => redelivered = Some(replacement),
            other => panic!("concurrent presentation returned {other:?}"),
        }
    }
    let rotated = rotated.expect("one presentation rotates");
    let redelivered = redelivered.expect("the other receives the successor");
    assert_eq!(rotated.digest, redelivered.digest);
    assert_eq!(rotated.generation, 1);

    let late = start + TimeDelta::seconds(7);
    match db
        .a
        .rotate_refresh_token(
            &digest(1),
            &family.token(1, 4, late),
            late,
            &Nothing,
            &Nothing,
        )
        .await
        .unwrap()
    {
        RefreshRotation::ReplayDetected(revoked) => {
            assert_eq!(revoked.revocation, Some(RefreshRevocation::TokenReplay));
            assert_eq!(revoked.revoked_at, Some(late));
        }
        other => panic!("a late replay returned {other:?}"),
    }
    assert!(matches!(
        db.a.rotate_refresh_token(
            &rotated.digest,
            &family.token(2, 5, late),
            late,
            &Nothing,
            &Nothing
        )
        .await
        .unwrap(),
        RefreshRotation::Invalid
    ));
    let rows = sqlx::query_file!("tests/queries/gateway/token_rows.sql")
        .fetch_all(db.a.pool())
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].consumed_at.is_some() && rows[0].replay_detected_at.is_some());
    assert!(!rows[0].delivered && rows[1].delivered);

    let retention = db.b.prune_refresh_state(late).await.unwrap();
    assert_eq!(retention.delivery_envelopes_deleted, 1);
    assert_eq!(
        (retention.tokens_deleted, retention.families_deleted),
        (0, 0)
    );
    let retention =
        db.b.prune_refresh_state(start + TimeDelta::days(8))
            .await
            .unwrap();
    assert_eq!(
        (retention.tokens_deleted, retention.families_deleted),
        (2, 1)
    );
    assert!(
        db.a.refresh_family(&family.family.id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_refused_append_rolls_the_rotation_back() {
    let db = TestDb::new().await;
    let start = now();
    let family = Family::new(start);
    db.a.create_refresh_family(&family.family, &family.token(0, 1, start))
        .await
        .unwrap();
    let at = start + TimeDelta::seconds(1);
    let refused =
        db.a.rotate_refresh_token(&digest(1), &family.token(1, 2, at), at, &Refused, &Nothing)
            .await
            .unwrap_err();
    assert!(matches!(refused, StoreError::Append(_)));
    let (token, stored) = db.a.refresh_grant(&digest(1)).await.unwrap().unwrap();
    assert!(token.consumed_at.is_none());
    assert_eq!(stored.current_generation, 0);
    assert!(db.a.refresh_grant(&digest(2)).await.unwrap().is_none());
    assert!(matches!(
        db.a.rotate_refresh_token(&digest(1), &family.token(1, 3, at), at, &Nothing, &Nothing)
            .await
            .unwrap(),
        RefreshRotation::Rotated { .. }
    ));
}

#[tokio::test]
async fn client_revocation_is_bound_to_its_client_and_idempotent() {
    let db = TestDb::new().await;
    let start = now();
    let family = Family::new(start);
    db.a.create_refresh_family(&family.family, &family.token(0, 1, start))
        .await
        .unwrap();
    let other_client = OAuthClientId::parse("other").unwrap();
    let foreign = RefreshBinding {
        oauth_client_id: &other_client,
        ..family.binding()
    };
    assert!(
        db.a.revoke_refresh_family(&digest(1), foreign, start)
            .await
            .unwrap()
            .is_none()
    );
    let revoked =
        db.a.revoke_refresh_family(&digest(1), family.binding(), start)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        revoked.revocation,
        Some(RefreshRevocation::ClientRevocation)
    );
    let again =
        db.b.revoke_refresh_family(&digest(1), family.binding(), start + TimeDelta::seconds(1))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(again.revoked_at, revoked.revoked_at);
    assert!(
        db.a.revoke_refresh_family(&digest(9), family.binding(), start)
            .await
            .unwrap()
            .is_none()
    );
}

fn control_plane(contexts: serde_json::Value) -> GatewayControlPlane {
    serde_json::from_value(serde_json::json!({
        "identityProviders": [],
        "authorizationServers": [],
        "profiles": [],
        "tenants": [],
        "workContexts": contexts,
        "policies": [],
        "dataLabels": [],
        "oidcClients": []
    }))
    .unwrap()
}

fn work_context(memberships: serde_json::Value) -> serde_json::Value {
    serde_json::json!([{
        "id": "mission",
        "tenant": "acme",
        "title": "Mission",
        "policyRevision": "r1",
        "outputPolicy": {
            "owner": { "kind": "group", "id": "operators" },
            "initialGrants": [{ "subject": { "kind": "group", "id": "auditors" }, "level": "read" }]
        },
        "memberships": memberships
    }])
}

fn revision(id: &str, byte: u8, control_plane: GatewayControlPlane) -> GatewayControlPlaneRevision {
    GatewayControlPlaneRevision {
        revision_id: GatewayControlPlaneRevisionId::parse(id).unwrap(),
        sha256: digest(byte).hex().to_owned(),
        source: GatewayControlPlaneRevisionSource::AdminApi,
        applied_at: now(),
        applied_by: PrincipalId::parse("admin").unwrap(),
        tenant: None,
        control_plane,
    }
}

fn object(kind: &str, id: &str, document: serde_json::Value) -> ControlObject {
    let serde_json::Value::Object(document) = document else {
        panic!("objects are JSON objects");
    };
    ControlObject {
        kind: ExtensionName::parse(kind).unwrap(),
        id: id.to_owned(),
        tenant: None,
        document,
        profile_policy_version: (kind == "profile").then(|| PolicyVersion::parse("p1").unwrap()),
    }
}

#[tokio::test]
async fn publications_replace_work_contexts_and_bind_artifact_upload_authority() {
    let db = TestDb::new().await;
    let tenant = TenantId::parse("acme").unwrap();
    let mission = WorkContextId::parse("mission").unwrap();
    let profile = GatewayProfileId::parse("operator").unwrap();
    assert!(db.a.active_control_revision().await.unwrap().is_none());
    assert!(
        db.a.artifact_upload_authority(&tenant, &mission, &profile)
            .await
            .unwrap()
            .is_none()
    );
    let objects = [
        object(
            "profile",
            "operator",
            serde_json::json!({ "id": "operator" }),
        ),
        object("policy", "p1", serde_json::json!({ "version": "p1" })),
    ];
    let first = revision(
        "gcp-first",
        1,
        control_plane(work_context(serde_json::json!([
            { "level": "contributor", "groups": ["operators"] }
        ]))),
    );
    db.a.publish_control_revision(
        ControlPublication {
            revision: &first,
            replaces: None,
            objects: &objects,
        },
        &Nothing,
    )
    .await
    .unwrap();
    assert_eq!(
        db.b.active_control_revision().await.unwrap(),
        Some(first.clone())
    );
    let context =
        db.b.work_context_by_key(deterministic_tenant_id("acme").unwrap(), "mission")
            .await
            .unwrap()
            .unwrap();
    assert_eq!(context.output_policy.initial_grants.len(), 1);
    assert_eq!(context.memberships.len(), 1);
    let bound =
        db.a.artifact_upload_authority(&tenant, &mission, &profile)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(bound.control_plane_digest, digest(1));
    assert_eq!(bound.policy_revision, PolicyVersion::parse("r1").unwrap());
    assert!(bound.profile_policy_digest.is_some());
    assert!(
        db.a.artifact_upload_authority(
            &tenant,
            &mission,
            &GatewayProfileId::parse("other").unwrap()
        )
        .await
        .unwrap()
        .unwrap()
        .profile_policy_digest
        .is_none()
    );

    // A publication built on a revision that is no longer active is refused.
    let stale = revision(
        "gcp-stale",
        2,
        control_plane(work_context(serde_json::json!([]))),
    );
    let second = revision(
        "gcp-second",
        3,
        control_plane(work_context(serde_json::json!([
            { "level": "owner", "groups": ["operators"] }
        ]))),
    );
    db.b.publish_control_revision(
        ControlPublication {
            revision: &second,
            replaces: Some(&first.revision_id),
            objects: &objects,
        },
        &Nothing,
    )
    .await
    .unwrap();
    assert!(matches!(
        db.a.publish_control_revision(
            ControlPublication {
                revision: &stale,
                replaces: Some(&first.revision_id),
                objects: &[],
            },
            &Nothing,
        )
        .await,
        Err(StoreError::ControlRevisionChanged)
    ));
    assert!(matches!(
        db.a.publish_control_revision(
            ControlPublication {
                revision: &first,
                replaces: Some(&second.revision_id),
                objects: &[],
            },
            &Nothing,
        )
        .await,
        Err(StoreError::ControlRevisionExists)
    ));
    assert!(matches!(
        db.a.publish_control_revision(
            ControlPublication {
                revision: &stale,
                replaces: Some(&second.revision_id),
                objects: &[],
            },
            &Refused,
        )
        .await,
        Err(StoreError::Append(_))
    ));
    assert_eq!(
        db.a.active_control_revision().await.unwrap(),
        Some(second.clone())
    );
    let rebound =
        db.a.artifact_upload_authority(&tenant, &mission, &profile)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(rebound.control_plane_digest, digest(3));
    assert_ne!(
        rebound.context_digest, bound.context_digest,
        "changed membership rules change the Work Context digest"
    );
    assert_eq!(rebound.profile_policy_digest, bound.profile_policy_digest);
}

#[tokio::test]
async fn task_routes_go_with_their_work_context() {
    let db = TestDb::new().await;
    let first = revision(
        "gcp-first",
        1,
        control_plane(work_context(serde_json::json!([]))),
    );
    db.a.publish_control_revision(
        ControlPublication {
            revision: &first,
            replaces: None,
            objects: &[],
        },
        &Nothing,
    )
    .await
    .unwrap();
    db.a.ensure_identity(
        "acme",
        "https://issuer.example#alice",
        "https://issuer.example",
        "alice",
        PrincipalKind::User,
    )
    .await
    .unwrap();
    let authority = digest(7);
    sqlx::query_file!(
        "tests/queries/gateway/insert_task_route.sql",
        format!("gtr_{}", "a".repeat(43)),
        deterministic_tenant_id("acme").unwrap() as TenantRow,
        deterministic_principal_id("acme", "https://issuer.example#alice").unwrap()
            as veoveo_platform_store::PrincipalId,
        deterministic_work_context_id("acme", "mission").unwrap() as WorkContextRow,
        authority.as_str(),
    )
    .execute(db.a.pool())
    .await
    .unwrap();
    let count = || {
        sqlx::query_file_scalar!("tests/queries/gateway/task_route_count.sql")
            .fetch_one(db.a.pool())
    };
    assert_eq!(count().await.unwrap(), 1);
    let second = revision("gcp-second", 2, control_plane(serde_json::json!([])));
    db.a.publish_control_revision(
        ControlPublication {
            revision: &second,
            replaces: Some(&first.revision_id),
            objects: &[],
        },
        &Nothing,
    )
    .await
    .unwrap();
    assert_eq!(count().await.unwrap(), 0);
}

#[tokio::test]
async fn registration_changes_wake_the_gateway_channel_and_liveness_stamps_do_not() {
    let db = TestDb::new().await;
    let mut changes = db.b.listen(vec![Channel::GATEWAY]);
    assert_eq!(
        changes.next().await.unwrap().unwrap(),
        ChangeSignal::Reconcile
    );
    let metadata = |version: &str| {
        Json(serde_json::json!({ "software_id": "observatory", "software_version": version }))
    };
    sqlx::query_file!(
        "tests/queries/gateway/insert_claim.sql",
        "observatory",
        "k".repeat(43),
    )
    .execute(db.a.pool())
    .await
    .unwrap();
    let token = digest(1);
    sqlx::query_file!(
        "tests/queries/gateway/insert_registration.sql",
        "observatory-1",
        "observatory",
        metadata("0.1.0") as _,
        token.as_str(),
        "observatory_1",
    )
    .execute(db.a.pool())
    .await
    .unwrap();
    let expect = |operation| {
        move |signal: Option<Result<ChangeSignal, StoreError>>| match signal.unwrap().unwrap() {
            ChangeSignal::Changed(change) => {
                assert_eq!(change.table, "registration");
                assert_eq!(change.operation, operation);
                assert_eq!(change.key.as_deref(), Some("observatory-1"));
            }
            other => panic!("expected a registration change, got {other:?}"),
        }
    };
    expect(ChangeOperation::Insert)(changes.next().await);
    sqlx::query_file!(
        "tests/queries/gateway/stamp_registration.sql",
        "observatory-1"
    )
    .execute(db.a.pool())
    .await
    .unwrap();
    sqlx::query_file!(
        "tests/queries/gateway/update_registration.sql",
        "observatory-1",
        metadata("0.2.0") as _,
    )
    .execute(db.a.pool())
    .await
    .unwrap();
    // The liveness stamp sent nothing, so the next signal is the metadata change.
    expect(ChangeOperation::Update)(changes.next().await);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), changes.next())
            .await
            .is_err()
    );
}
