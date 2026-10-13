//! Wire format, schema and admission checks for the registration declarations and grants.
use serde_json::json;
use veoveo_mcp_ext_registration::*;
use veoveo_types::{
    LocalToolName, PermissionName, PromptName, ResourceScheme, ResourceTemplateUri, ResourceUri,
    ServerSlug,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, veoveo_types::Vocabulary)]
enum ClockPermission {
    #[vocabulary(rename = "clock.read")]
    Read,
    #[vocabulary(rename = "clock.events.write")]
    EventsWrite,
    #[vocabulary(rename = "clock.admin")]
    Admin,
}

impl PermissionDefinition for ClockPermission {
    fn resource(self) -> &'static str {
        match self {
            Self::Read | Self::Admin => "clock.authority",
            Self::EventsWrite => "clock.event",
        }
    }
    fn access(self) -> Access {
        match self {
            Self::Read => Access::Read,
            Self::EventsWrite | Self::Admin => Access::Write,
        }
    }
    fn risks(self) -> &'static [Risk] {
        match self {
            Self::Admin => &[Risk::Administer],
            _ => &[],
        }
    }
}

fn slug() -> ServerSlug {
    ServerSlug::parse("clock").unwrap()
}
fn scheme() -> ResourceScheme {
    ResourceScheme::parse("clock").unwrap()
}
fn tool(name: &str) -> LocalToolName {
    LocalToolName::parse(name).unwrap()
}
fn template(value: &str) -> ResourceTemplateUri {
    ResourceTemplateUri::new(value).unwrap()
}

fn authorization() -> AuthorizationDeclaration {
    AuthorizationBuilder::<ClockPermission>::new()
        .tool(tool("resolve_time"), ClockPermission::Read)
        .tool(tool("create_event"), ClockPermission::EventsWrite)
        .tool(tool("activate_release"), ClockPermission::Admin)
        .resource(
            ResourceUri::new("clock://authorities/current").unwrap(),
            ClockPermission::Read,
        )
        .resource_template(template("clock://events/{event}"), ClockPermission::Read)
        .resource_template(template("ui://clock/{view}"), ClockPermission::Read)
        .prompt(
            PromptName::parse("plan_window").unwrap(),
            ClockPermission::Read,
        )
        .build(&slug(), &scheme())
        .unwrap()
}

#[test]
fn authorization_declaration_wire_format() {
    let wire = serde_json::to_value(authorization()).unwrap();
    assert_eq!(
        wire,
        json!({
            "permissions": [
                {"permission": "clock.read", "resource": "clock.authority", "access": "read"},
                {"permission": "clock.events.write", "resource": "clock.event", "access": "write"},
                {"permission": "clock.admin", "resource": "clock.authority", "access": "write", "risks": ["administer"]}
            ],
            "requirements": {
                "tools": {
                    "activate_release": "clock.admin",
                    "create_event": "clock.events.write",
                    "resolve_time": "clock.read"
                },
                "resources": {"clock://authorities/current": "clock.read"},
                "resourceTemplates": {
                    "clock://events/{event}": "clock.read",
                    "ui://clock/{view}": "clock.read"
                },
                "prompts": {"plan_window": "clock.read"}
            }
        })
    );
    let back: AuthorizationDeclaration = serde_json::from_value(wire).unwrap();
    assert_eq!(back, authorization());
    back.check(&slug(), &scheme()).unwrap();
}

#[test]
fn required_permission_lookup() {
    let declaration = authorization();
    let required = |target| {
        declaration
            .required(target)
            .map(|permission| permission.permission.as_str().to_owned())
    };
    let create_event = tool("create_event");
    assert_eq!(
        required(RequirementTarget::Tool(&create_event)).as_deref(),
        Some("clock.events.write")
    );
    let event = ResourceUri::new("clock://events/abc").unwrap();
    assert_eq!(
        required(RequirementTarget::Resource(&event)).as_deref(),
        Some("clock.read")
    );
    let unknown = ResourceUri::new("clock://elsewhere/abc").unwrap();
    assert_eq!(required(RequirementTarget::Resource(&unknown)), None);
    let undeclared = tool("undeclared");
    assert_eq!(required(RequirementTarget::Tool(&undeclared)), None);
}

#[test]
fn ambiguous_template_matches_require_nothing() {
    let declaration = AuthorizationBuilder::<ClockPermission>::new()
        .resource_template(template("clock://events/{event}"), ClockPermission::Read)
        .resource_template(
            template("clock://{kind}/{id}"),
            ClockPermission::EventsWrite,
        )
        .build(&slug(), &scheme())
        .unwrap();
    let event = ResourceUri::new("clock://events/abc").unwrap();
    assert!(
        declaration
            .required(RequirementTarget::Resource(&event))
            .is_none()
    );
}

#[test]
fn declarations_outside_the_namespace_are_rejected() {
    let mut wire = serde_json::to_value(authorization()).unwrap();
    wire["permissions"][0]["permission"] = json!("map.read");
    wire["requirements"]["tools"]["resolve_time"] = json!("map.read");
    let declaration: AuthorizationDeclaration = serde_json::from_value(wire).unwrap();
    assert!(matches!(
        declaration.check(&slug(), &scheme()),
        Err(DeclarationError::PermissionOutsideNamespace { .. })
    ));

    let mut wire = serde_json::to_value(authorization()).unwrap();
    wire["permissions"][0]["resource"] = json!("map.layer");
    let declaration: AuthorizationDeclaration = serde_json::from_value(wire).unwrap();
    assert!(matches!(
        declaration.check(&slug(), &scheme()),
        Err(DeclarationError::ResourceOutsideNamespace { .. })
    ));

    let mut wire = serde_json::to_value(authorization()).unwrap();
    wire["requirements"]["resources"] = json!({"map://layers/a": "clock.read"});
    let declaration: AuthorizationDeclaration = serde_json::from_value(wire).unwrap();
    assert!(matches!(
        declaration.check(&slug(), &scheme()),
        Err(DeclarationError::ResourceOutsideScheme { .. })
    ));

    let mut wire = serde_json::to_value(authorization()).unwrap();
    wire["requirements"]["tools"]["resolve_time"] = json!("clock.undeclared");
    let declaration: AuthorizationDeclaration = serde_json::from_value(wire).unwrap();
    assert!(matches!(
        declaration.check(&slug(), &scheme()),
        Err(DeclarationError::UndeclaredPermission { .. })
    ));

    let mut wire = serde_json::to_value(authorization()).unwrap();
    let first = wire["permissions"][0].clone();
    wire["permissions"].as_array_mut().unwrap().push(first);
    let declaration: AuthorizationDeclaration = serde_json::from_value(wire).unwrap();
    assert!(matches!(
        declaration.check(&slug(), &scheme()),
        Err(DeclarationError::DuplicatePermission { .. })
    ));

    let mut wire = serde_json::to_value(authorization()).unwrap();
    wire["requirements"]["prompts"]["plan_window"] = json!("clock.events.write");
    let declaration: AuthorizationDeclaration = serde_json::from_value(wire).unwrap();
    assert!(matches!(
        declaration.check(&slug(), &scheme()),
        Err(DeclarationError::PromptRequiresWrite { .. })
    ));
}

#[test]
fn wire_admission_rejects_unknown_vocabulary() {
    let mut wire = serde_json::to_value(authorization()).unwrap();
    wire["permissions"][0]["access"] = json!("execute");
    assert!(serde_json::from_value::<AuthorizationDeclaration>(wire).is_err());
    let mut wire = serde_json::to_value(authorization()).unwrap();
    wire["permissions"][2]["risks"] = json!(["radioactive"]);
    assert!(serde_json::from_value::<AuthorizationDeclaration>(wire).is_err());
    let mut wire = serde_json::to_value(authorization()).unwrap();
    wire["roles"] = json!({});
    assert!(serde_json::from_value::<AuthorizationDeclaration>(wire).is_err());
}

#[test]
fn builder_rejects_conflicting_requirements() {
    let result = AuthorizationBuilder::<ClockPermission>::new()
        .tool(tool("resolve_time"), ClockPermission::Read)
        .tool(tool("resolve_time"), ClockPermission::Admin)
        .build(&slug(), &scheme());
    assert!(matches!(
        result,
        Err(DeclarationError::ConflictingRequirement { .. })
    ));
    let other = ServerSlug::parse("map").unwrap();
    assert!(
        AuthorizationBuilder::<ClockPermission>::new()
            .build(&other, &scheme())
            .is_err()
    );
}

fn registration() -> RegistrationDeclaration {
    RegistrationDeclaration {
        slug: slug(),
        uri_scheme: scheme(),
        version: ServerVersion::new("0.1.0").unwrap(),
        endpoint: ServerEndpoint::new(url::Url::parse("https://clock.example/clock/mcp").unwrap())
            .unwrap(),
    }
}

#[test]
fn registration_declaration_wire_format() {
    let wire = serde_json::to_value(registration()).unwrap();
    assert_eq!(
        wire,
        json!({
            "slug": "clock",
            "uriScheme": "clock",
            "version": "0.1.0",
            "endpoint": "https://clock.example/clock/mcp"
        })
    );
    assert_eq!(
        serde_json::from_value::<RegistrationDeclaration>(wire).unwrap(),
        registration()
    );
}

#[test]
fn registration_values_are_admitted_once() {
    let mut wire = serde_json::to_value(registration()).unwrap();
    for (field, value) in [
        ("endpoint", json!("ftp://clock/mcp")),
        ("endpoint", json!("https://user:secret@clock.example/mcp")),
        ("endpoint", json!("http://clock.example/mcp")),
        ("endpoint", json!("https://clock.example/mcp?token=x")),
        ("version", json!("")),
        ("version", json!("1 0")),
        ("slug", json!("Clock")),
    ] {
        let original = wire[field].clone();
        wire[field] = value;
        assert!(
            serde_json::from_value::<RegistrationDeclaration>(wire.clone()).is_err(),
            "{field}"
        );
        wire[field] = original;
    }
    wire["publicKey"] = json!({});
    assert!(serde_json::from_value::<RegistrationDeclaration>(wire).is_err());
}

/// RFC 8037 Appendix A.3: the thumbprint of the example Ed25519 key.
const RFC8037_X: &str = "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo";
const RFC8037_THUMBPRINT: &str = "kPrK_qmxVWaYVA9wwBF6Iuo3vVzz7TxHCTwXBygrS4k";

fn rfc8037_key() -> ServerPublicKey {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(RFC8037_X)
        .unwrap();
    ServerPublicKey::from_bytes(bytes.try_into().unwrap())
}

#[test]
fn server_keys_are_jwks_identified_by_their_thumbprint() {
    let key = rfc8037_key();
    assert_eq!(key.thumbprint().as_str(), RFC8037_THUMBPRINT);
    let wire = serde_json::to_value(ServerKeySet::new(key)).unwrap();
    assert_eq!(
        wire,
        json!({"keys": [{
            "kty": "OKP", "crv": "Ed25519", "x": RFC8037_X,
            "kid": RFC8037_THUMBPRINT, "alg": "EdDSA", "use": "sig"
        }]})
    );
    assert_eq!(
        serde_json::from_value::<ServerKeySet>(wire.clone())
            .unwrap()
            .key(),
        &key
    );
    let jwk = wire["keys"][0].clone();
    let with = |member: &str, value: serde_json::Value| {
        let mut jwk = jwk.clone();
        jwk[member] = value;
        json!({"keys": [jwk]})
    };
    for rejected in [
        json!({"keys": []}),
        json!({"keys": [jwk.clone(), jwk.clone()]}),
        with("kid", json!("not-the-thumbprint")),
        with("kty", json!("RSA")),
        with("crv", json!("X25519")),
        with("alg", json!("ES256")),
        with("use", json!("enc")),
        with("x", json!("Bwc")),
        with("d", json!("private")),
    ] {
        assert!(
            serde_json::from_value::<ServerKeySet>(rejected.clone()).is_err(),
            "{rejected}"
        );
    }
    // The Gateway's key set rotates several keys, each named by its thumbprint.
    let next = ServerPublicKey::from_bytes([7; 32]);
    let rotation = json!({"keys": [
        serde_json::to_value(Ed25519Jwk::new(key)).unwrap(),
        serde_json::to_value(Ed25519Jwk::new(next)).unwrap()
    ]});
    let keys: Ed25519JwkSet = serde_json::from_value(rotation).unwrap();
    assert_eq!(keys.get(&next.thumbprint()), Some(&next));
    let duplicate = json!({"keys": [jwk.clone(), jwk.clone()]});
    assert!(serde_json::from_value::<Ed25519JwkSet>(duplicate).is_err());
    let mut named = jwk.clone();
    named["kid"] = json!("gateway-1");
    assert!(serde_json::from_value::<Ed25519JwkSet>(json!({"keys": [named]})).is_err());
}

fn metadata() -> ClientMetadata {
    ClientMetadata::new(rfc8037_key(), registration(), authorization()).unwrap()
}

const STATEMENT: &str = "eyJhbGciOiJFZERTQSJ9.eyJpc3MiOiJjbG9jayJ9.c2lnbmF0dXJl";

fn registration_request() -> ClientRegistrationRequest {
    ClientRegistrationRequest {
        metadata: metadata(),
        software_statement: SoftwareStatement::new(STATEMENT).unwrap(),
    }
}

#[test]
fn registration_requests_are_rfc7591_client_metadata() {
    let wire = serde_json::to_value(registration_request()).unwrap();
    assert_eq!(wire["token_endpoint_auth_method"], "private_key_jwt");
    assert_eq!(wire["grant_types"], json!([]));
    assert_eq!(wire["response_types"], json!([]));
    assert_eq!(wire["jwks"]["keys"][0]["kid"], RFC8037_THUMBPRINT);
    assert_eq!(wire["software_statement"], STATEMENT);
    assert_eq!(wire["software_id"], "clock");
    assert_eq!(wire["software_version"], "0.1.0");
    assert_eq!(wire["veoveo_uri_scheme"], "clock");
    assert_eq!(
        wire["veoveo_mcp_endpoint"],
        "https://clock.example/clock/mcp"
    );
    // OAuth documents spell members in snake_case; MCP keeps camelCase.
    let mut snake = serde_json::to_value(authorization()).unwrap();
    let templates = snake["requirements"]
        .as_object_mut()
        .unwrap()
        .remove("resourceTemplates")
        .unwrap();
    snake["requirements"]["resource_templates"] = templates;
    assert_eq!(wire["veoveo_authorization"], snake);
    for member in ["ai.veoveo/registration", "ai.veoveo/authorization"] {
        assert!(wire.get(member).is_none(), "{member}");
    }
    assert_eq!(
        serde_json::from_value::<ClientRegistrationRequest>(wire.clone()).unwrap(),
        registration_request()
    );

    // RFC 7591 §2: unknown client metadata is ignored.
    let mut extended = wire.clone();
    extended["client_name"] = json!("Clock");
    assert_eq!(
        serde_json::from_value::<ClientRegistrationRequest>(extended).unwrap(),
        registration_request()
    );
    // Veoveo members admit only their own fields.
    let mut extended = wire.clone();
    extended["veoveo_authorization"]["owner"] = json!("someone");
    assert!(serde_json::from_value::<ClientRegistrationRequest>(extended).is_err());
    let mut unsigned = wire;
    unsigned["software_statement"] = json!("not a jwt");
    assert!(serde_json::from_value::<ClientRegistrationRequest>(unsigned).is_err());
}

#[test]
fn client_metadata_checks_every_declaration() {
    metadata().check().unwrap();
    let mut granted = metadata();
    granted.grant_types = vec![GrantType::ClientCredentials];
    assert!(granted.check().is_err());

    let mut elsewhere = registration();
    elsewhere.slug = ServerSlug::parse("map").unwrap();
    assert!(ClientMetadata::new(rfc8037_key(), elsewhere, authorization()).is_err());

    // Kernel and PostgreSQL schemas are not server namespaces.
    for reserved in ["kernel", "identity", "public", "pg-stat"] {
        let mut registration = registration();
        registration.slug = ServerSlug::parse(reserved).unwrap();
        registration.uri_scheme = ResourceScheme::parse(reserved).unwrap();
        let mut metadata = metadata();
        metadata.registration = registration;
        assert!(
            matches!(
                metadata.check(),
                Err(DeclarationError::Invalid {
                    field: "software_id",
                    ..
                })
            ),
            "{reserved}"
        );
    }
}

fn information() -> serde_json::Value {
    let mut wire = serde_json::to_value(registration_request()).unwrap();
    let object = wire.as_object_mut().unwrap();
    for (member, value) in [
        ("client_id", json!("clock-0b6f")),
        ("client_id_issued_at", json!(1_791_763_200)),
        ("registration_access_token", json!("rat-secret")),
        (
            "registration_client_uri",
            json!("https://gateway.example/oauth/register/clock-0b6f"),
        ),
        (
            "veoveo_access_tokens",
            json!({
                "issuer": "https://gateway.example",
                "jwks": {"keys": [{
                    "kty": "OKP", "crv": "Ed25519", "x": RFC8037_X,
                    "kid": RFC8037_THUMBPRINT, "alg": "EdDSA", "use": "sig"
                }]},
                "resource": "https://clock.example/clock/mcp"
            }),
        ),
        (
            "veoveo_database",
            json!({
                "url": "postgres://postgres.internal:5432/veoveo?sslmode=verify-full",
                "username": "clock_0b6f",
                "password": "hunter2"
            }),
        ),
    ] {
        object.insert(member.to_owned(), value);
    }
    wire
}

#[test]
fn client_information_carries_token_trust_and_database_login() {
    let received: ClientInformation = serde_json::from_value(information()).unwrap();
    received.check().unwrap();
    assert_eq!(received.client_id.as_str(), "clock-0b6f");
    assert_eq!(
        received.access_tokens.issuer.as_str(),
        "https://gateway.example"
    );
    assert_eq!(received.metadata, metadata());
    let debug = format!("{received:?}");
    assert!(!debug.contains("hunter2"));
    assert!(!debug.contains("rat-secret"));
    assert_eq!(
        serde_json::to_value(&received).unwrap(),
        information(),
        "the response round-trips, secrets included"
    );
    for issuer in ["veoveo-internal", "http://gateway.example"] {
        let mut opaque_issuer = information();
        opaque_issuer["veoveo_access_tokens"]["issuer"] = json!(issuer);
        let received: ClientInformation = serde_json::from_value(opaque_issuer).unwrap();
        assert!(received.check().is_err(), "the issuer must be an https URL");
    }
    let mut elsewhere = information();
    elsewhere["veoveo_access_tokens"]["resource"] = json!("https://other.example/mcp");
    let received: ClientInformation = serde_json::from_value(elsewhere).unwrap();
    assert!(
        received.check().is_err(),
        "the audience is the registered endpoint"
    );
    let mut without_database = information();
    without_database
        .as_object_mut()
        .unwrap()
        .remove("veoveo_database");
    let received: ClientInformation = serde_json::from_value(without_database).unwrap();
    assert!(received.database.is_none());
}

#[test]
fn database_grants_name_one_postgres_database_and_role() {
    let received: ClientInformation = serde_json::from_value(information()).unwrap();
    let database = received.database.unwrap();
    assert_eq!(database.username.as_str(), "clock_0b6f");
    assert_eq!(database.url.as_url().path(), "/veoveo");
    for (member, value) in [
        ("url", json!("mysql://database.internal:3306/veoveo")),
        ("url", json!("postgres://postgres.internal:5432/")),
        ("url", json!("postgres://postgres.internal/veoveo/extra")),
        (
            "url",
            json!("postgres://clock:secret@postgres.internal/veoveo"),
        ),
        ("url", json!("postgres://postgres.internal/veoveo#fragment")),
        ("username", json!("Clock")),
        ("username", json!("0clock")),
        ("username", json!("clock-0b6f")),
        ("username", json!("c".repeat(64))),
        ("namespace", json!("veoveo")),
    ] {
        let mut wire = information();
        wire["veoveo_database"][member] = value;
        assert!(
            serde_json::from_value::<ClientInformation>(wire).is_err(),
            "{member}"
        );
    }
    assert!(DatabaseIdentifier::new("_".repeat(63)).is_ok());
    assert!(DatabaseUrl::try_from("postgresql://[::1]:5432/veoveo".to_owned()).is_ok());
}

#[test]
fn registration_errors_use_rfc7591_codes() {
    let error = RegistrationErrorResponse {
        error: RegistrationErrorCode::UnapprovedSoftwareStatement,
        error_description: Some("namespace clock is pinned to another key".into()),
    };
    assert_eq!(
        serde_json::to_value(&error).unwrap(),
        json!({
            "error": "unapproved_software_statement",
            "error_description": "namespace clock is pinned to another key"
        })
    );
    assert!(
        serde_json::from_value::<RegistrationErrorResponse>(json!({"error": "teapot"})).is_err()
    );
}

#[test]
fn authorization_server_metadata_urls_follow_rfc8414() {
    let root = url::Url::parse("https://gateway.example").unwrap();
    assert_eq!(
        AuthorizationServerMetadata::url_for(&root).as_str(),
        "https://gateway.example/.well-known/oauth-authorization-server"
    );
    let tenant = url::Url::parse("https://gateway.example/tenant/").unwrap();
    assert_eq!(
        AuthorizationServerMetadata::url_for(&tenant).as_str(),
        "https://gateway.example/.well-known/oauth-authorization-server/tenant"
    );
}

#[test]
fn authorization_server_metadata_names_its_key_set() {
    let metadata = |jwks_uri: Option<&str>| {
        let mut wire = json!({
            "issuer": "https://gateway.example",
            "registration_endpoint": "https://gateway.example/oauth/register",
            "token_endpoint": "https://gateway.example/oauth/token"
        });
        if let Some(uri) = jwks_uri {
            wire["jwks_uri"] = json!(uri);
        }
        serde_json::from_value::<AuthorizationServerMetadata>(wire).unwrap()
    };
    let published = metadata(Some("https://gateway.example/oauth/jwks.json"));
    assert_eq!(
        published.key_set_url().unwrap().as_str(),
        "https://gateway.example/oauth/jwks.json"
    );
    assert_eq!(
        serde_json::to_value(&published).unwrap()["jwks_uri"],
        "https://gateway.example/oauth/jwks.json"
    );
    assert!(
        metadata(Some("http://127.0.0.1:8080/oauth/jwks.json"))
            .key_set_url()
            .is_ok()
    );
    for rejected in [
        None,
        Some("http://gateway.example/oauth/jwks.json"),
        Some("https://gateway.example/oauth/jwks.json?kid=1"),
        Some("ftp://gateway.example/jwks.json"),
    ] {
        assert!(metadata(rejected).key_set_url().is_err(), "{rejected:?}");
    }
}

#[test]
fn granted_permissions_are_an_rfc9396_detail() {
    let detail = PermissionsDetail::new(
        [
            PermissionName::parse("clock.read").unwrap(),
            PermissionName::parse("clock.events.write").unwrap(),
        ]
        .into(),
    );
    let wire = serde_json::to_value(&detail).unwrap();
    assert_eq!(
        wire,
        json!({
            "type": PERMISSIONS_DETAIL_TYPE,
            "actions": ["clock.events.write", "clock.read"]
        })
    );
    detail.check(&slug()).unwrap();
    assert!(detail.check(&ServerSlug::parse("map").unwrap()).is_err());
    for rejected in [
        json!({"type": "https://example.com/payment", "actions": ["clock.read"]}),
        json!({"type": PERMISSIONS_DETAIL_TYPE, "actions": ["clock.read"], "locations": []}),
        json!({"type": PERMISSIONS_DETAIL_TYPE, "actions": ["read"]}),
    ] {
        assert!(serde_json::from_value::<PermissionsDetail>(rejected).is_err());
    }
}

#[test]
fn published_schemas_admit_the_wire_examples() {
    fn validator<T: schemars::JsonSchema>() -> jsonschema::Validator {
        let schema = serde_json::to_value(schemars::schema_for!(T)).unwrap();
        assert_eq!(
            schema["$schema"], "https://json-schema.org/draft/2020-12/schema",
            "schemas use JSON Schema 2020-12"
        );
        jsonschema::validator_for(&schema).unwrap()
    }
    for (validator, example) in [
        (
            validator::<AuthorizationDeclaration>(),
            serde_json::to_value(authorization()).unwrap(),
        ),
        (
            validator::<RegistrationDeclaration>(),
            serde_json::to_value(registration()).unwrap(),
        ),
        (
            validator::<ClientRegistrationRequest>(),
            serde_json::to_value(registration_request()).unwrap(),
        ),
        (validator::<ClientInformation>(), information()),
        (
            validator::<PermissionsDetail>(),
            serde_json::to_value(PermissionsDetail::new(
                [PermissionName::parse("clock.read").unwrap()].into(),
            ))
            .unwrap(),
        ),
        (
            validator::<RegistrationErrorResponse>(),
            json!({"error": "invalid_client_metadata", "error_description": "tool undeclared"}),
        ),
    ] {
        assert!(validator.is_valid(&example), "{example}");
    }
    let validator = validator::<ClientRegistrationRequest>();
    for (pointer, value) in [
        (
            "/veoveo_authorization/permissions/0/access",
            json!("execute"),
        ),
        ("/jwks/keys/0/kty", json!("RSA")),
        ("/token_endpoint_auth_method", json!("client_secret_basic")),
    ] {
        let mut invalid = serde_json::to_value(registration_request()).unwrap();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert!(!validator.is_valid(&invalid), "{pointer}");
    }
}

#[test]
fn advertised_declarations_round_trip_through_capabilities() {
    let mut capabilities = rmcp::model::ServerCapabilities::default();
    mcp::advertise(&mut capabilities, &registration(), &authorization());
    let advertised = mcp::advertised(&capabilities).unwrap().unwrap();
    assert_eq!(advertised.registration, registration());
    assert_eq!(advertised.authorization, authorization());
    assert_eq!(
        mcp::advertised(&rmcp::model::ServerCapabilities::default()).unwrap(),
        None
    );
}

#[test]
fn tool_annotations_must_agree_with_permissions() {
    let declaration = authorization();
    let annotated = |name: &str, read_only: Option<bool>, destructive: Option<bool>| {
        let mut tool = rmcp::model::Tool::new(
            name.to_owned(),
            "test tool",
            std::sync::Arc::new(serde_json::Map::new()),
        );
        let mut annotations = rmcp::model::ToolAnnotations::default();
        annotations.read_only_hint = read_only;
        annotations.destructive_hint = destructive;
        tool.annotations = Some(annotations);
        tool
    };
    mcp::check_tool_annotations(&declaration, &[annotated("resolve_time", Some(true), None)])
        .unwrap();
    assert!(matches!(
        mcp::check_tool_annotations(&declaration, &[annotated("create_event", Some(true), None)]),
        Err(DeclarationError::ReadOnlyToolRequiresWrite { .. })
    ));
    assert!(matches!(
        mcp::check_tool_annotations(
            &declaration,
            &[annotated("activate_release", Some(false), Some(true))]
        ),
        Err(DeclarationError::DestructiveToolWithoutRisk { .. })
    ));
}

#[test]
fn plain_http_is_allowed_only_for_local_hosts() {
    for local in [
        "http://localhost:8800/mcp",
        "http://time.localhost:8800/mcp",
        "http://127.0.0.1:8800/mcp",
        "http://[::1]:8800/mcp",
        "http://host.openshell.internal:8800/mcp",
    ] {
        assert!(
            ServerEndpoint::new(url::Url::parse(local).unwrap()).is_ok(),
            "{local}"
        );
        assert!(ResourceIndicator::parse(local).is_ok(), "{local}");
    }
    for remote in [
        "http://time.example/mcp",
        "http://10.0.0.5:8800/mcp",
        "ftp://localhost/mcp",
    ] {
        assert!(
            ServerEndpoint::new(url::Url::parse(remote).unwrap()).is_err(),
            "{remote}"
        );
        assert!(ResourceIndicator::parse(remote).is_err(), "{remote}");
    }
    let local_issuer =
        veoveo_types::TokenIssuer::parse("http://host.openshell.internal:8080").unwrap();
    assert!(veoveo_mcp_ext_registration::oauth::issuer_url(&local_issuer).is_ok());
    let remote_issuer = veoveo_types::TokenIssuer::parse("http://gateway.example").unwrap();
    assert!(veoveo_mcp_ext_registration::oauth::issuer_url(&remote_issuer).is_err());
}
