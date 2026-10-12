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
| OAuth 2.0 Dynamic Client Registration, RFC 7591 | Registration request and response. The installation's registration secret is the initial access token. A server registers `jwks`, `token_endpoint_auth_method` `private_key_jwt`, empty `grant_types` and `response_types`, a `software_statement` it signs itself, and the three declarations as client metadata. Errors use the §3.2.2 codes. |
| OAuth 2.0 Dynamic Client Registration Management, RFC 7592 | Read (`GET`), update (`PUT`) and delete (`DELETE`) at `registration_client_uri` with the registration access token. Renewing a lease is an ordinary update. |
| Bearer tokens, RFC 6750 | Initial access token, registration access token and forwarded access token travel in `Authorization: Bearer`. |
| JWT, RFC 7519, with JWS, RFC 7515, and EdDSA over Ed25519, RFC 8037 | Software statements and forwarded access tokens. `alg` is `EdDSA`; no other algorithm verifies. |
| JWK and JWK Set, RFC 7517, and JWK Thumbprint, RFC 7638 | Ed25519 `OKP` keys with `kid`, `alg: EdDSA` and `use: sig`. A server's `kid` is its key's thumbprint. |
| JWT assertions, RFC 7523 §3 | Claim rules for the software statement: `iss` and `sub` name the server, `aud` is the URL the request is sent to, `jti` is unique, and the lifetime is at most 60 seconds. |
| JWT access tokens, RFC 9068 | The forwarded identity: `typ: at+jwt`, `aud` names the server, `client_id` names the Gateway, and `scope`, `groups` and `roles` restate the caller. |
| Rich Authorization Requests, RFC 9396 | `authorization_details` carries one entry of type `https://veoveo.ai/oauth/permissions` whose `actions` are the granted permissions. |
| Authorization server metadata, RFC 8414 | A server reads `issuer` and `registration_endpoint` from the Gateway's well-known metadata. |
| MCP `2026-07-28` | A server advertises the declarations in Discover under `capabilities.extensions`. |
| JSON Schema 2020-12 | Schemas for every request, response and declaration are generated from the Rust types by Schemars 1.2. |
| Veoveo module manifests | `ai.veoveo/module` is the `veoveo-modules` manifest, admitted exactly as a compiled module declaration. |

Every Veoveo addition uses the extension point of the standard it extends, under a
collision-resistant name:

| Extension point | Names |
|---|---|
| RFC 7591 client metadata | `ai.veoveo/registration`, `ai.veoveo/authorization`, `ai.veoveo/module` |
| RFC 7591 response members | `ai.veoveo/lease`, `ai.veoveo/access-tokens`, `ai.veoveo/database` |
| RFC 9396 authorization details type | `https://veoveo.ai/oauth/permissions` |
| JWT private claims | `ai.veoveo/invocation`, `ai.veoveo/resource-invocation`, `ai.veoveo/artifact-upload` |
| MCP `capabilities.extensions` | `ai.veoveo/registration`, `ai.veoveo/authorization`, `ai.veoveo/module` |

Following RFC 7591, a receiver ignores client metadata it does not understand. Each
Veoveo member admits only its own declared fields and refuses unknown ones.

## Declarations

### `ai.veoveo/registration`

The server's slug, the URI scheme of its resources, its build version and the URL
where the Gateway reaches its MCP endpoint.

```json
{
  "slug": "time",
  "uriScheme": "time",
  "version": "0.1.0",
  "endpoint": "http://vv-mcp-time:8800/mcp"
}
```

The version holds 1 to 64 ASCII letters, digits, dots, underscores, plus signs or
hyphens. The endpoint uses `http` or `https`, names a host and carries no credentials,
query or fragment.

### `ai.veoveo/authorization`

The server's permissions and the permission each tool, resource, resource template and
prompt requires.

```json
{
  "permissions": [
    { "permission": "time.read", "resource": "time.authority", "access": "read" },
    { "permission": "time.events.write", "resource": "time.event", "access": "write" },
    { "permission": "time.admin", "resource": "time.authority", "access": "write", "risks": ["administer"] }
  ],
  "requirements": {
    "tools": { "resolve_time": "time.read", "create_temporal_event": "time.events.write" },
    "resourceTemplates": { "time://events/{event}": "time.read" },
    "prompts": { "plan_window": "time.read" }
  }
}
```

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

### `ai.veoveo/module`

The module manifest for a server that owns tables. A server without tables omits it.

```json
{
  "name": "time",
  "ownership": [
    { "kind": "tablePrefix", "name": "time_" },
    { "kind": "functionPrefix", "name": "fn::time::" }
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
JSON form. Discover advertises the registration and authorization declarations in
full, so the Gateway compares what a server registered with what it serves.

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
  "ai.veoveo/registration": { "slug": "time", "uriScheme": "time", "version": "0.1.0", "endpoint": "http://vv-mcp-time:8800/mcp" },
  "ai.veoveo/authorization": { "permissions": [], "requirements": {} },
  "ai.veoveo/module": { "name": "time", "ownership": [], "migrations": [] },
  "software_statement": "eyJ0eXAiOiJKV1QiLCJhbGciOiJFZERTQSIsImtpZCI6Ii4uLiJ9.eyIuLi4iOiIuLi4ifQ.c2ln"
}
```

In an OpenShell environment the server holds a placeholder for the registration secret,
and the proxy substitutes the real value only on requests to the registration endpoint.

`jwks` holds exactly one key, and its `kid` is the key's RFC 7638 thumbprint. The
Gateway admits a request in this order and stops at the first failure:

1. The bearer is the installation's registration secret, or the answer is HTTP 401.
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
  "ai.veoveo/registration": { "...": "..." },
  "ai.veoveo/authorization": { "...": "..." },
  "ai.veoveo/module": { "...": "..." },
  "software_statement": "<as sent>",
  "ai.veoveo/lease": { "expiresAt": "2026-10-11T12:00:30Z", "renewEverySeconds": 10 },
  "ai.veoveo/access-tokens": {
    "issuer": "veoveo-internal",
    "jwks": { "keys": [{ "kty": "OKP", "crv": "Ed25519", "x": "...", "kid": "gateway-1", "alg": "EdDSA", "use": "sig" }] }
  },
  "ai.veoveo/database": {
    "endpoint": "ws://surrealdb:8000/",
    "namespace": "veoveo",
    "database": "platform",
    "username": "time_runtime",
    "password": "<secret>"
  }
}
```

The response repeats the registered metadata unchanged, and the registration client
rejects a response that differs from what it sent. `ai.veoveo/database` appears when
the server registered a module and the Gateway issued its user. The user has
database-wide access and never administrator rights. `ai.veoveo/access-tokens` names
the issuer and keys of the access tokens the Gateway forwards to this server.
`registration_client_uri` uses `http` or `https`.

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
| `jti` | a fresh UUID |

The Gateway verifies the signature with the key in `jwks`, requires that key to equal
the pinned key when the namespace has one, and requires the statement's metadata to
equal the request's. It tolerates 5 seconds of clock difference at either end of the
lifetime. A failure is `invalid_software_statement`, except a foreign key for a pinned
namespace, which is `unapproved_software_statement`.

## Lease Renewal

A registration is a lease. `ai.veoveo/lease` gives the expiry and the interval at
which the server renews. The server renews with an RFC 7592 update that repeats its
metadata:

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
| Missing or wrong initial access token or registration access token | 401 | none |
| Malformed body or a declaration outside the namespace | 400 | `invalid_client_metadata` |
| Malformed, stale, misaddressed or mismatched software statement | 400 | `invalid_software_statement` |
| Namespace pinned to another key | 400 | `unapproved_software_statement` |

An error body is an RFC 7591 §3.2.2 object. `error_description` states the exact
reason and names declared identifiers only, never request contents or secrets.

## Forwarded Identity

The Gateway authorizes each request with installation roles over the target server's
declared permissions, as the [policy evaluator](../../platform/policy/DESIGN.md#registered-server-decisions)
describes. It then forwards the request with a fresh RFC 9068 access token signed by
one of the keys in `ai.veoveo/access-tokens`:

```json
{ "typ": "at+jwt", "alg": "EdDSA", "kid": "gateway-1" }
```

```json
{
  "iss": "veoveo-internal",
  "sub": "https://idp.example#alice",
  "aud": "time",
  "client_id": "veoveo-gateway",
  "iat": 1791763200,
  "nbf": 1791763200,
  "exp": 1791763260,
  "jti": "0b6f6a0e-6c55-4d4e-9c43-8f1f6f1b2a10",
  "scope": "time:use",
  "groups": ["tenant-members"],
  "authorization_details": [
    { "type": "https://veoveo.ai/oauth/permissions", "actions": ["time.events.write", "time.read"] }
  ],
  "ai.veoveo/invocation": {
    "profile": "default",
    "actor": { "id": "https://idp.example#alice", "kind": "user", "...": "..." },
    "authority": { "work_context": "development", "tenant": "dev", "membership": "contributor", "...": "..." },
    "request_context": { "...": "..." }
  }
}
```

`ai.veoveo/invocation` carries the profile, the actor, the invocation authority with
its tenant, Work Context and membership, and the signed request context when one
exists. `authorization_details` is omitted when nothing is granted. A server verifies
the token and rejects it when:

- `typ` is neither `at+jwt` nor `application/at+jwt`, or `alg` is not `EdDSA`;
- `kid` names no key from `ai.veoveo/access-tokens`, or the signature fails;
- `iss` differs from the registered issuer, `aud` is not the server's slug, or `exp`
  and `nbf` place the current time outside the token's lifetime, with no leeway;
- `client_id` is not `veoveo-gateway`, or `sub`, `groups`, `roles` and `scope`
  disagree with the embedded actor;
- the token carries more than one authorization details entry, or a granted
  permission lies outside the server's namespace;
- the request context disagrees with the actor or authority, or the token outlives
  the caller's own access token.

An ordinary request's token lives at most 60 seconds, and a token that opens
`subscriptions/listen` at most 15 minutes. Both end no later than the caller's own
access token.

The server checks the required permission again with
`GatewayInternalIdentity::grants`. Per-object rules, such as an owner-only event,
stay with the server. `veoveo-mcp-contract` issues and verifies these tokens with
`GatewayInternalTokenIssuer::issue_granted` and `GatewayInternalTokenVerifier`, and
`GatewayInternalTrustBundle::from_key_set` turns `ai.veoveo/access-tokens` into the
verifier's key set. The hosted runtime admits the issuer `veoveo-internal`.

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
let trust = GatewayInternalTrustBundle::from_key_set(&registration.access_tokens().jwks)?;
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
tests cover the pinned key, metadata, audience and lifetime. The forwarded token tests
in `veoveo-mcp-contract` cover the token type, the single authorization detail, the
namespace of granted permissions and agreement between standard and Veoveo claims.
