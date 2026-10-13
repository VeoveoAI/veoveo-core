use super::*;
use chrono::TimeDelta;

#[derive(Deserialize)]
struct Fixture {
    name: String,
    actor: Principal,
    authority: InvocationAuthority,
    request_context: GatewayRequestContext,
}
/// A request carrying `context`, through the OAuth client its access token names.
fn forwarded_with(
    actor: &Principal,
    authority: InvocationAuthority,
    context: GatewayRequestContext,
) -> ForwardedRequest {
    ForwardedRequest {
        profile: GatewayProfileId::parse("operator").unwrap(),
        server: ServerSlug::parse("computers").unwrap(),
        audience: tests::resource("computers"),
        client: context.access_token.oauth_client_id.clone(),
        actor: actor.clone(),
        authority,
        permissions: Default::default(),
        request_context: Some(context),
        expires_at: Utc::now() + TimeDelta::seconds(60),
    }
}

fn fixtures() -> Vec<Fixture> {
    serde_json::from_str(include_str!(
        "../../../../testing/fixtures/gateway-request-context.json"
    ))
    .unwrap()
}

#[test]
fn all_invocation_modes_preserve_signed_source_and_session_context() {
    let issuer = tests::issuer();
    let verifier = tests::verifier("computers");
    for fixture in fixtures() {
        let issued = issuer
            .issue(forwarded_with(
                &fixture.actor,
                fixture.authority,
                fixture.request_context.clone(),
            ))
            .unwrap();
        let received = verifier.verify(&issued.bearer_token).unwrap();
        assert_eq!(received.actor, fixture.actor, "{}", fixture.name);
        assert_eq!(
            received.request_context,
            Some(fixture.request_context),
            "{}",
            fixture.name
        );
    }
}

#[test]
fn source_expiry_caps_assertions_and_cannot_be_extended_by_a_signed_context() {
    let mut fixture = fixtures().remove(0);
    fixture.request_context.access_token.expires_at = Utc::now() + TimeDelta::seconds(10);
    let key = tests::signing_key();
    let issuer = tests::issuer();
    let verifier = tests::verifier("computers");
    let issue = |context| {
        issuer.issue(forwarded_with(
            &fixture.actor,
            fixture.authority.clone(),
            context,
        ))
    };
    let issued = issue(fixture.request_context.clone()).unwrap();
    assert_eq!(
        issued.identity.expires_at,
        fixture.request_context.access_token.expires_at
    );
    verifier.verify(&issued.bearer_token).unwrap();
    let mut claims = AccessTokenClaims::from_identity(&issued.identity);
    claims.standard.exp += 1;
    let mut header = Header::new(Algorithm::EdDSA);
    header.typ = Some(ACCESS_TOKEN_TYPE.into());
    header.kid = Some(key.key_id().to_owned());
    let forged = encode(
        &header,
        &claims,
        &EncodingKey::from_ed_der(&key.private_key_der),
    )
    .unwrap();
    assert!(matches!(
        verifier.verify(&forged),
        Err(InternalTokenError::InvalidRequestContext)
    ));
    fixture.request_context.access_token.expires_at = Utc::now() - TimeDelta::seconds(1);
    assert!(matches!(
        issue(fixture.request_context),
        Err(InternalTokenError::ExpiredDelegation)
    ));
}

#[test]
fn signed_context_rejects_mismatched_actor_tenant_scope_and_provenance() {
    for fixture in fixtures() {
        for mismatch in [
            "subject", "issuer", "tenant", "context", "scope", "mode", "actor",
        ] {
            let mut context = fixture.request_context.clone();
            let mut actor = fixture.actor.clone();
            match mismatch {
                "subject" => {
                    context.access_token.subject = crate::TokenSubject::parse("foreign").unwrap()
                }
                "issuer" => {
                    context.access_token.issuer =
                        TokenIssuer::parse("https://foreign.example").unwrap()
                }
                "tenant" => {
                    context.principal.tenant =
                        Some(veoveo_types::TenantId::parse("foreign").unwrap())
                }
                "context" => {
                    context.access_token.work_context =
                        veoveo_types::WorkContextId::parse("foreign").unwrap()
                }
                "scope" => {
                    context
                        .access_token
                        .scopes
                        .insert(veoveo_types::ScopeName::parse("admin:use").unwrap());
                }
                "mode" => {
                    context.access_token.invocation_mode = if context.access_token.invocation_mode
                        == veoveo_types::InvocationMode::Direct
                    {
                        veoveo_types::InvocationMode::Automated
                    } else {
                        veoveo_types::InvocationMode::Direct
                    }
                }
                _ => actor.id = PrincipalId::parse("foreign").unwrap(),
            }
            assert!(
                context.validate_for(&actor, &fixture.authority).is_err(),
                "{} {mismatch}",
                fixture.name
            );
        }
    }
}

#[test]
fn forwarded_tokens_reject_other_types_and_invalid_execution_attribution() {
    let fixture = fixtures()
        .into_iter()
        .find(|fixture| {
            fixture
                .request_context
                .access_token
                .managed_execution
                .is_some()
        })
        .unwrap();
    let key = tests::signing_key();
    let issuer = tests::issuer();
    let verifier = tests::verifier("computers");
    let issued = issuer
        .issue(forwarded_with(
            &fixture.actor,
            fixture.authority.clone(),
            fixture.request_context.clone(),
        ))
        .unwrap();
    let mut segments = issued.bearer_token.split('.');
    let issued_header: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(segments.next().unwrap()).unwrap()).unwrap();
    assert_eq!(issued_header["typ"], ACCESS_TOKEN_TYPE);
    let original: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(segments.next().unwrap()).unwrap()).unwrap();
    let sign = |header: &Header, claims: &serde_json::Value| {
        encode(
            header,
            claims,
            &EncodingKey::from_ed_der(&key.private_key_der),
        )
        .unwrap()
    };
    let mut header = Header::new(Algorithm::EdDSA);
    header.typ = Some(ACCESS_TOKEN_TYPE.into());
    header.kid = Some(key.key_id().to_owned());
    verifier.verify(&sign(&header, &original)).unwrap();
    // RFC 9068 §4: `at+jwt` or its full media type, and nothing else.
    let mut media_type = header.clone();
    media_type.typ = Some("application/at+jwt".into());
    verifier.verify(&sign(&media_type, &original)).unwrap();
    for typ in [Some("JWT"), Some("application/jwt"), None] {
        let mut other = header.clone();
        other.typ = typ.map(str::to_owned);
        assert!(matches!(
            verifier.verify(&sign(&other, &original)),
            Err(InternalTokenError::UnsupportedTokenType)
        ));
    }
    for variant in 0..7 {
        let mut claims = original.clone();
        let invocation = INVOCATION_CLAIM;
        match variant {
            0 => {
                claims.as_object_mut().unwrap().remove("client_id");
            }
            1 => claims["client_id"] = serde_json::json!("console"),
            2 => claims["groups"] = serde_json::json!(["foreign"]),
            3 => {
                claims[invocation]["request_context"]["format"] =
                    serde_json::json!("https://veoveo.ai/formats/gateway-request-context")
            }
            4 => {
                claims[invocation]["request_context"]["access_token"]["managed_execution"]["generation"] =
                    serde_json::json!(0)
            }
            5 => {
                claims[invocation]["request_context"]["access_token"]["managed_execution"]["dispatch_epoch"] =
                    serde_json::json!(-1)
            }
            _ => {
                claims[invocation]["request_context"]["access_token"]["managed_agent"] =
                    serde_json::json!({"instance":"fixture-agent","generation":1,"epoch":2});
            }
        }
        assert!(
            verifier.verify(&sign(&header, &claims)).is_err(),
            "{variant}"
        );
    }
    let mut value = serde_json::to_value(&fixture.request_context).unwrap();
    value["access_token"]["session_family"] =
        serde_json::json!("019b7b88-7f03-7123-8123-abcdefabcdef");
    let context: GatewayRequestContext = serde_json::from_value(value).unwrap();
    assert!(
        context
            .validate_for(&fixture.actor, &fixture.authority)
            .is_err()
    );
}

#[test]
fn signed_execution_projection_preserves_frozen_audit_actor_shape() {
    let fixture = fixtures()
        .into_iter()
        .find(|fixture| {
            fixture
                .request_context
                .access_token
                .managed_execution
                .is_some()
        })
        .unwrap();
    let projection = fixture
        .request_context
        .access_token
        .managed_execution
        .clone()
        .unwrap();
    let audit = fixture
        .request_context
        .audit_context(
            &fixture.actor,
            &fixture.authority,
            &GatewayProfileId::parse("operator").unwrap(),
        )
        .unwrap();
    assert_eq!(audit.actor.managed_agent, Some(projection));
    let actor = serde_json::to_value(audit.actor).unwrap();
    assert_eq!(actor["managed_agent"]["dispatch_epoch"], 2);
    assert!(actor.get("managed_execution").is_none());
}

#[test]
fn the_token_names_the_client_the_caller_used() {
    let fixture = fixtures().remove(0);
    let issuer = tests::issuer();
    let verifier = tests::verifier("computers");
    let request = forwarded_with(
        &fixture.actor,
        fixture.authority.clone(),
        fixture.request_context.clone(),
    );
    let identity = verifier
        .verify(&issuer.issue(request.clone()).unwrap().bearer_token)
        .unwrap();
    assert_eq!(
        identity.client,
        fixture.request_context.access_token.oauth_client_id
    );
    let elsewhere = ForwardedRequest {
        client: crate::OAuthClientId::parse("another-client").unwrap(),
        ..request
    };
    assert!(matches!(
        issuer.issue(elsewhere),
        Err(InternalTokenError::InvalidRequestContext)
    ));
}
