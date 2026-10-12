# Server Registration Protocol

This document is the normative protocol between an MCP server and the Veoveo Gateway.
A server registers itself, declares what it owns and which permission each of its
targets requires, and renews its registration while it can serve traffic. The Gateway
forwards each admitted request with the caller's granted permissions, and the server
checks them again. Neither the Gateway nor the installation configuration names a
server, so every server enters the catalog through this protocol.

The crate `veoveo-mcp-ext-registration` holds the typed declarations, the OAuth
request and response types, the software statement, the granted-permissions detail and
the registration client that servers run. The forwarded access token is issued and
verified by `veoveo-mcp-contract`, described under [Forwarded Identity](#forwarded-identity).

## Standards And Protocols

| Standard | Profile |
|---|---|
| OAuth 2.0 Dynamic Client Registration, RFC 7591 | Registration request and response. The installation's registration secret is the initial access token. A server registers `jwks`, `token_endpoint_auth_method` `private_key_jwt`, empty `grant_types` and `response_types`, its slug as `software_id`, its build as `software_version`, a `software_statement` it signs itself, and Veoveo members for its declarations. Errors use the §3.2.2 codes. |
| OAuth 2.0 Dynamic Client Registration Management, RFC 7592 | Read (`GET`), update (`PUT`) and delete (`DELETE`) at `registration_client_uri` with the registration access token. Renewing a lease is an ordinary update. |
| OAuth 2.0, RFC 6749 §8.2 | Registration members without a standard equivalent take the `veoveo_` vendor prefix in snake_case. |
| Bearer tokens, RFC 6750 | Initial access token, registration access token and forwarded access token travel in `Authorization: Bearer`. Every 401 carries `WWW-Authenticate: Bearer`, with `error="invalid_token"` once a token was sent. |
| JWT, RFC 7519, with JWS, RFC 7515, and EdDSA over Ed25519, RFC 8037 | Software statements and forwarded access tokens. `alg` is `EdDSA`; no other algorithm verifies. Times in OAuth documents are NumericDate seconds. |
| JWK and JWK Set, RFC 7517, and JWK Thumbprint, RFC 7638 | Ed25519 `OKP` keys with `kid`, `alg: EdDSA` and `use: sig`. Every `kid`, a server's or the Gateway's, is the key's RFC 7638 thumbprint. |
| JWT assertions, RFC 7523 §3 | Claim rules for the software statement: `iss` and `sub` name the server, `aud` is the URL the request is sent to, `jti` is a UUIDv7, and the lifetime is at most 60 seconds. |
| JWT access tokens, RFC 9068 | The forwarded identity: `typ: at+jwt`, `iss` is the Gateway's HTTPS issuer URL, `aud` is the target server's resource URL, `client_id` names the OAuth client the caller used, `jti` is a UUIDv7, and `scope`, `groups` and `roles` restate the caller. |
| Resource indicators, RFC 8707 | A forwarded token's `aud` is an absolute URL: the target server's registered MCP endpoint. |
| OAuth 2.0 Token Exchange, RFC 8693 §4.1 | The `act` claim of a forwarded token names the Gateway as the party acting for the caller. Nested actor chains are refused. |
| Rich Authorization Requests, RFC 9396 | `authorization_details` carries one entry of type `https://veoveo.ai/oauth/permissions` whose `actions` are the granted permissions. |
| Authorization server metadata, RFC 8414 | A server reads `issuer` and `registration_endpoint` from the Gateway's well-known metadata. The issuer is the forwarded tokens' `iss`. |
| MCP `2026-07-28` | A server advertises the declarations in Discover under `capabilities.extensions`, with camelCase members. |
| JSON Schema 2020-12 | Schemas for every request, response and declaration are generated from the Rust types by Schemars 1.2. |
| Veoveo module manifests | `veoveo_module` is the `veoveo-modules` manifest, admitted exactly as a compiled module declaration. |

Every Veoveo addition uses the extension point of the standard it extends:

| Extension point | Names |
|---|---|
| RFC 7591 client metadata, RFC 6749 §8.2 vendor prefix | `veoveo_uri_scheme`, `veoveo_mcp_endpoint`, `veoveo_authorization`, `veoveo_module` |
| RFC 7591 response members, RFC 6749 §8.2 vendor prefix | `veoveo_lease`, `veoveo_access_tokens`, `veoveo_database` |
| RFC 9396 authorization details type | `https://veoveo.ai/oauth/permissions` |
| JWT public claims, RFC 7519 §4.2 collision-resistant names | `https://veoveo.ai/claims/invocation`, `https://veoveo.ai/claims/resource-invocation`, `https://veoveo.ai/claims/artifact-upload` |
| MCP `capabilities.extensions` | `ai.veoveo/registration`, `ai.veoveo/authorization`, `ai.veoveo/module` |

Following RFC 7591, a receiver ignores client metadata it does not understand. Each
Veoveo member admits only its own declared fields and refuses unknown ones. OAuth
documents spell members in snake_case, and MCP advertisements in camelCase.

## Declarations

### Identity

A server registers its slug as `software_id`, its build as `software_version`, the URI
scheme of its resources as `veoveo_uri_scheme` and the URL where the Gateway reaches
its MCP endpoint as `veoveo_mcp_endpoint`. MCP Discover advertises the same values as
`ai.veoveo/registration`:

```json
{
  "slug": "time",
  "uriScheme": "time",
  "version": "0.1.0",
  "endpoint": "https://time.internal.example/mcp"
}
```

The version holds 1 to 64 ASCII letters, digits, dots, underscores, plus signs or
hyphens. The endpoint uses `https`, or `http` on a local host as the
[conventions](../../docs/CONVENTIONS.md#http-paths) allow, names a host and carries no
credentials, query or fragment.

### Authorization

The server's permissions and the permission each tool, resource, resource template and
prompt requires. The registration member `veoveo_authorization` spells it in snake_case:

```json
{
  "permissions": [
    { "permission": "time.read", "resource": "time.authority", "access": "read" },
    { "permission": "time.events.write", "resource": "time.event", "access": "write" },
    { "permission": "time.admin", "resource": "time.authority", "access": "write", "risks": ["administer"] }
  ],
  "requirements": {
    "tools": { "resolve_time": "time.read", "create_temporal_event": "time.events.write" },
    "resource_templates": { "time://events/{event}": "time.read" },
    "prompts": { "plan_window": "time.read" }
  }
}
```

MCP Discover advertises the same declaration as `ai.veoveo/authorization`, where the
requirements member is `resourceTemplates`.

`access` is exactly one of `read` or `write`. `risks` holds any of `sensitive`,
`destructive`, `billed`, `physical` and `administer`, and an empty set is omitted.
A server builds the declaration from a closed Rust enum that implements
`PermissionDefinition`, and `AuthorizationBuilder` derives every entry from that enum.
The same requirement table drives the server's own handler checks.

The Gateway rejects a declaration when any of these hold:

- a permission or resource type does not start with the server's slug and a dot;
- a permission is declared twice, or a requirement names an undeclared permission;
- a resource or template URI uses neither the server's scheme nor `ui://{slug}/`;
- a template uses an RFC 6570 operator, since only simple `{variable}` expressions
  match concrete URIs;
- a prompt requires a `write` permission;
- a tool annotated `readOnlyHint` requires a `write` permission, or a tool annotated
  `destructiveHint` requires a permission without the `destructive` risk.

A target the declaration omits is denied. An exact resource entry takes precedence
over templates, and a URI matched by templates that require different permissions
requires none and is denied.

### Module

`veoveo_module` is the module manifest for a server that owns tables. A server without
tables omits it.

```json
{
  "name": "time",
  "ownership": [
    { "kind": "table_prefix", "name": "time_" },
    { "kind": "function_prefix", "name": "fn::time::" }
  ],
  "requires": [{ "module": "identity" }],
  "migrations": [
    { "version": 0, "name": "current", "sql": "DEFINE TABLE time_event SCHEMAFULL;" }
  ]
}
```

The module is named after the slug. Its table, table-prefix and analyzer claims start
with `{slug}_` and its function claims with `fn::{slug}::`, writing each hyphen in the
slug as an underscore. The Gateway rebuilds the manifest with
`ModuleManifest::admit`, which applies the checked constructors and
`ModuleSetup::builder` that compiled declarations use. A server builds its manifest
from its compiled declaration with `ModuleManifest::from_setup`.

MCP Discover advertises `ModuleSummary` under `ai.veoveo/module` instead of the full
manifest: the name, the latest migration version and the SHA-256 of the manifest's
JSON form. Discover advertises the identity and authorization declarations in full, so
the Gateway compares what a server registered with what it serves.

## Namespace Claims

The first registration of a slug claims its namespace and pins the server's public
key to it. For slug `time` the namespace is the slug itself, the `time://` scheme, the
`ui://time/` App prefix, tables starting `time_`, functions under `fn::time::` and
permissions and resource types under `time.`. Every later registration of that slug,
including a new server version, must be signed by the pinned key. A registration signed
by another key is rejected with `unapproved_software_statement`. Releasing a namespace
is an administrative action outside this protocol.

## Registration

The server discovers the endpoint from the Gateway's RFC 8414 metadata at
`/.well-known/oauth-authorization-server`, then sends:

```http
POST /oauth/register HTTP/1.1
Authorization: Bearer <installation registration secret>
Content-Type: application/json

{
  "jwks": { "keys": [{ "kty": "OKP", "crv": "Ed25519", "x": "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo",
                       "kid": "kPrK_qmxVWaYVA9wwBF6Iuo3vVzz7TxHCTwXBygrS4k", "alg": "EdDSA", "use": "sig" }] },
  "token_endpoint_auth_method": "private_key_jwt",
  "grant_types": [],
  "response_types": [],
  "software_id": "time",
  "software_version": "0.1.0",
  "veoveo_uri_scheme": "time",
  "veoveo_mcp_endpoint": "https://time.internal.example/mcp",
  "veoveo_authorization": { "permissions": [], "requirements": {} },
  "veoveo_module": { "name": "time", "ownership": [], "migrations": [] },
  "software_statement": "eyJ0eXAiOiJKV1QiLCJhbGciOiJFZERTQSIsImtpZCI6Ii4uLiJ9.eyIuLi4iOiIuLi4ifQ.c2ln"
}
```

In an OpenShell environment the server holds a placeholder for the registration secret,
and the proxy substitutes the real value only on requests to the registration endpoint.

`jwks` holds exactly one key, and its `kid` is the key's RFC 7638 thumbprint. The
Gateway admits a request in this order and stops at the first failure:

1. The bearer is the installation's registration secret, or the answer is HTTP 401
   with `WWW-Authenticate: Bearer`.
2. The body decodes into `ClientRegistrationRequest`.
3. `ClientMetadata::check` passes: empty grant and response types, the authorization
   declaration within the namespace, and the module within its prefixes.
4. `statement::verify_statement` passes against the pinned key, if the namespace has
   one, and the registration endpoint URL.
5. The module manifest passes `ModuleManifest::admit`.

The Gateway then pins the namespace, applies the module's migrations with its own
database credentials, creates the server's database user and answers HTTP 201:

```json
{
  "client_id": "time-0b6f",
  "client_id_issued_at": 1791763200,
  "registration_access_token": "<opaque>",
  "registration_client_uri": "https://gateway.example/oauth/register/time-0b6f",
  "jwks": { "keys": ["..."] },
  "token_endpoint_auth_method": "private_key_jwt",
  "grant_types": [],
  "response_types": [],
  "software_id": "time",
  "software_version": "0.1.0",
  "veoveo_uri_scheme": "time",
  "veoveo_mcp_endpoint": "https://time.internal.example/mcp",
  "veoveo_authorization": { "...": "..." },
  "veoveo_module": { "...": "..." },
  "software_statement": "<as sent>",
  "veoveo_lease": { "expires_at": 1791763230, "renew_every_seconds": 10 },
  "veoveo_access_tokens": {
    "issuer": "https://gateway.example",
    "jwks": { "keys": [{ "kty": "OKP", "crv": "Ed25519", "x": "...", "kid": "<thumbprint>", "alg": "EdDSA", "use": "sig" }] },
    "resource": "https://time.internal.example/mcp"
  },
  "veoveo_database": {
    "endpoint": "ws://surrealdb:8000/",
    "namespace": "veoveo",
    "database": "platform",
    "username": "time_runtime",
    "password": "<secret>"
  }
}
```

The response repeats the registered metadata unchanged, and the registration client
rejects a response that differs from what it sent. `veoveo_database` appears when the
server registered a module and the Gateway issued its user. The user has database-wide
access and never administrator rights. `veoveo_access_tokens` names the issuer, keys
and audience of the access tokens the Gateway forwards to this server. The issuer is
the Gateway's issuer URL, the value its RFC 8414 metadata announces, and the
audience is the registered MCP endpoint. Every URL here uses `https`, or `http` on a
local host.

### Software Statement

The software statement proves that the registering process holds the private key in
`jwks` and binds that key to every declaration. It is a JWT whose header carries
`alg: EdDSA` and the key's thumbprint as `kid`, and whose claims are the request's
client metadata together with:

| Claim | Value |
|---|---|
| `iss`, `sub` | the server's slug |
| `aud` | the URL the request is sent to: the registration endpoint, or `registration_client_uri` for an update |
| `iat`, `exp` | issue and expiry time, at most 60 seconds apart |
| `jti` | a fresh UUIDv7 |

The Gateway verifies the signature with the key in `jwks`, requires that key to equal
the pinned key when the namespace has one, and requires the statement's metadata to
equal the request's. It tolerates 5 seconds of clock difference at either end of the
lifetime. A failure is `invalid_software_statement`, except a foreign key for a pinned
namespace, which is `unapproved_software_statement`.

## Lease Renewal

A registration is a lease. `veoveo_lease` gives the expiry as NumericDate seconds and
the interval at which the server renews. The server renews with an RFC 7592 update
that repeats its metadata:

```http
PUT /oauth/register/time-0b6f HTTP/1.1
Authorization: Bearer <registration access token>
Content-Type: application/json

{ "client_id": "time-0b6f", "jwks": { "...": "..." }, "...": "...", "software_statement": "<signed for this URL>" }
```

The Gateway admits an update with the same steps as a registration, using the
registration access token in place of the initial access token, and answers HTTP 200
with the client information and a new expiry. It may rotate the registration access
token, and the client adopts the returned one. An update with changed declarations
takes the full admission of a new registration under the pinned key. The installation
sets the lease timings in its `registration.lease` section: renewal every 10 seconds
and expiry after 30 by default, and expiry must exceed the renewal interval.

When a lease expires, the Gateway removes the server from routing and answers its
registration access token with HTTP 401. The registration client then registers again
with the initial access token.

## Read And Removal

`GET` at `registration_client_uri` returns the current client information with HTTP
200. `DELETE` removes the registration and answers HTTP 204. A server deletes its
registration when it shuts down, and the registration client treats HTTP 401 on delete
as already removed.

## Errors

| Condition | HTTP | Body |
|---|---|---|
| Missing or wrong initial access token or registration access token | 401 with `WWW-Authenticate: Bearer` | none |
| Malformed body or a declaration outside the namespace | 400 | `invalid_client_metadata` |
| Malformed, stale, misaddressed or mismatched software statement | 400 | `invalid_software_statement` |
| Namespace pinned to another key | 400 | `unapproved_software_statement` |

An error body is an RFC 7591 §3.2.2 object. `error_description` states the exact
reason and names declared identifiers only, never request contents or secrets.

## Forwarded Identity

The Gateway authorizes each request with installation roles over the target server's
declared permissions, as the [policy evaluator](../../platform/policy/DESIGN.md#registered-server-decisions)
describes. It then forwards the request with a fresh RFC 9068 access token signed by
one of the keys in `veoveo_access_tokens`:

```json
{ "typ": "at+jwt", "alg": "EdDSA", "kid": "<RFC 7638 thumbprint>" }
```

```json
{
  "iss": "https://gateway.example",
  "sub": "https://idp.example#alice",
  "aud": "https://time.internal.example/mcp",
  "client_id": "console",
  "act": { "sub": "https://gateway.example" },
  "iat": 1791763200,
  "nbf": 1791763200,
  "exp": 1791763260,
  "jti": "0199d4a1-7c00-7b3e-9f2a-6c1d2e3f4a5b",
  "scope": "time:use",
  "groups": ["tenant-members"],
  "authorization_details": [
    { "type": "https://veoveo.ai/oauth/permissions", "actions": ["time.events.write", "time.read"] }
  ],
  "https://veoveo.ai/claims/invocation": {
    "profile": "default",
    "server": "time",
    "actor": { "id": "https://idp.example#alice", "kind": "user", "...": "..." },
    "authority": { "work_context": "development", "tenant": "dev", "membership": "contributor", "...": "..." },
    "request_context": { "...": "..." }
  }
}
```

`sub` is the caller and `client_id` the OAuth client the caller used to reach the
Gateway. The Gateway is the party acting for the caller, so `act` names it by its
issuer URL. `aud` is the target server's resource URL, its registered MCP endpoint, and
the invocation claim names the server's slug. The invocation claim also carries the
profile, the actor, the invocation authority with its tenant, Work Context and
membership, and the signed request context when one exists. RFC 7519 §4.2 asks for
collision-resistant names for claims outside the IANA registry, and every registered
server reads the invocation claim, so it uses a URI in the `veoveo.ai` domain rather
than a private name. `authorization_details` is omitted when nothing is granted. A
server verifies the token and rejects it when:

- `typ` is neither `at+jwt` nor `application/at+jwt`, or `alg` is not `EdDSA`;
- `kid` names no key from `veoveo_access_tokens`, or the signature fails;
- `iss` differs from the registered issuer, `aud` differs from the registered
  resource, the invocation claim names another server, or `exp` and `nbf` place the
  current time outside the token's lifetime, with no leeway;
- `jti` is not a UUIDv7;
- `act` is missing, carries anything but `sub`, or names another party than `iss`;
- `sub`, `groups`, `roles` and `scope` disagree with the embedded actor, or
  `client_id` differs from the client in the request context;
- the token carries more than one authorization details entry, or a granted
  permission lies outside the server's namespace;
- the request context disagrees with the actor or authority, or the token outlives
  the caller's own access token.

A rejected request receives HTTP 401 with `WWW-Authenticate: Bearer`, adding
`error="invalid_token"` when a token was sent. An ordinary request's token lives at
most 60 seconds, and a token that opens `subscriptions/listen` at most 15 minutes.
Both end no later than the caller's own access token.

The server checks the required permission again with
`GatewayInternalIdentity::grants`. Per-object rules, such as an owner-only event,
stay with the server. `veoveo-mcp-contract` issues these tokens with
`GatewayInternalTokenIssuer::issue` from a `ForwardedRequest`, and
`GatewayInternalTokenVerifier::from_access_tokens` verifies them against the issuer,
keys and resource in `veoveo_access_tokens`. The hosted runtime takes the same value
through `HostedServerBuilder::internal_trust`.

## Registration Client

The `client` feature provides the client every server runs:

```rust
let key = KeyDirectory::new(&args.key_directory).load_or_generate()?;
let endpoint = RegistrationClient::discover(&http, &gateway_issuer).await?;
let metadata = ClientMetadata::new(
    *key.public_key(),
    registration,
    AuthorizationBuilder::<TimePermission>::new()
        .tool(LocalToolName::parse("resolve_time")?, TimePermission::Read)
        .build(&slug, &scheme)?,
    Some(ModuleDeclaration::new(ModuleManifest::from_setup(&time_module())?)),
)?;
let client = RegistrationClient::new(http, endpoint, registration_secret, key, metadata)?;
let registration = client.register().await?;
let database = registration.database();
let server = HostedServer::for_domain::<TimeDomain>()
    .deployment(&deployment, false)?
    .internal_trust(registration.access_tokens())?;
tokio::spawn(async move { client.maintain(registration, shutdown_signal()).await });
```

`KeyDirectory` generates the Ed25519 key on first start and stores it as PKCS#8 in
`server-key.p8`, readable only by its owner and written atomically. Restarts reuse the
key, so the server keeps its namespace. `RegistrationClient::maintain` renews at the
lease interval until the shutdown future completes, then deletes the registration. A
transport failure or HTTP 5xx is retried at the next interval, and any other failure
ends the loop with its error. The caller supplies the `reqwest::Client` and installs a
rustls crypto provider before reaching the Gateway over HTTPS.

## Crate Features

| Feature | Adds |
|---|---|
| none (`contract`) | Declarations, the OAuth request and response types, `PermissionsDetail`, JWK types and their schemas |
| `signing` | `ServerKeyPair` and the software statement, on AWS-LC |
| `client` | `KeyDirectory` and `RegistrationClient`, on reqwest and Tokio |
| `mcp` | Advertising and reading the declarations in MCP capabilities, and the tool annotation check |

Defaults select `mcp`, `signing` and `client`. The workspace depends on the crate with
default features disabled, and each consumer selects what it uses.

## Qualification

`tests/wire.rs` fixes the wire form of every declaration, request, response, error and
authorization detail, and validates the examples against the generated JSON Schemas.
It checks the RFC 7638 thumbprint against RFC 8037 Appendix A. `tests/client.rs` runs
the registration client against an in-process endpoint that applies the admission
steps above: registration, read, renewal with token rotation, re-registration after
expiry, removal, the pinned key and the initial access token. The software statement
tests cover the pinned key, metadata, audience and lifetime. The forwarded token
tests in `veoveo-mcp-contract` cover the token type, issuer, audience, actor, `jti`,
the single authorization detail, the namespace of granted permissions and agreement
between standard and Veoveo claims.
