# Naming And Format Conventions

Every name, identifier, format, wire field and protocol element in a Veoveo repository
follows a published standard. Where no standard applies, it follows a rule in this
document. An extension uses the extension mechanism and naming style of the standard
it extends. A name or format that fits neither is a defect.

## Standards And Protocols

| Standard | Governs |
|---|---|
| [RFC 6749](https://www.rfc-editor.org/rfc/rfc6749) §8.2 | Vendor-specific OAuth parameter names |
| [RFC 7591](https://www.rfc-editor.org/rfc/rfc7591), [RFC 7592](https://www.rfc-editor.org/rfc/rfc7592) | Client registration documents and their errors |
| [RFC 7519](https://www.rfc-editor.org/rfc/rfc7519), [RFC 9068](https://www.rfc-editor.org/rfc/rfc9068), [RFC 8693](https://www.rfc-editor.org/rfc/rfc8693), [RFC 9396](https://www.rfc-editor.org/rfc/rfc9396) | JWT claims, access tokens, the `act` claim and `authorization_details` |
| [RFC 8414](https://www.rfc-editor.org/rfc/rfc8414), [RFC 9728](https://www.rfc-editor.org/rfc/rfc9728), [RFC 8707](https://www.rfc-editor.org/rfc/rfc8707), [RFC 8615](https://www.rfc-editor.org/rfc/rfc8615) | Issuer and resource identifiers, well-known URIs |
| [RFC 7638](https://www.rfc-editor.org/rfc/rfc7638) | Key identifiers |
| [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457), [RFC 6750](https://www.rfc-editor.org/rfc/rfc6750) | HTTP error bodies and bearer challenges |
| [RFC 9562](https://www.rfc-editor.org/rfc/rfc9562), [RFC 3339](https://www.rfc-editor.org/rfc/rfc3339) | Identifiers and timestamps |
| [RFC 3986](https://www.rfc-editor.org/rfc/rfc3986), [RFC 6570](https://www.rfc-editor.org/rfc/rfc6570), [RFC 7595](https://www.rfc-editor.org/rfc/rfc7595) | URIs, URI templates and URI schemes |
| [JSON Schema 2020-12](https://json-schema.org/draft/2020-12/) | Schema identifiers and vocabularies |
| [OCI Image Spec](https://github.com/opencontainers/image-spec) annotations and digests | Image labels and `sha256:` digests |
| [OpenTelemetry semantic conventions](https://opentelemetry.io/docs/specs/semconv/) | `service.name` and resource attributes |

## JSON Casing

| Document family | Member casing |
|---|---|
| Veoveo HTTP API bodies, configuration files and Veoveo formats | `camelCase` |
| OAuth, OpenID Connect and JWT documents, including objects nested in a Veoveo claim | `snake_case`, as their specifications use |
| PostgreSQL rows and columns, and formats whose canonical bytes are a stored record, such as Audit records and blocks | `snake_case`, the database's naming |

Enum values are `snake_case` in every family. One JSON object uses one casing; an
extension nested inside another family's document follows that document's casing. A
stored record is not returned as an API body; the API defines its own camelCase type.

## Extension Names

| Host standard | Form | Example |
|---|---|---|
| OAuth parameters and client metadata | `veoveo_<snake_name>` | `veoveo_authorization` |
| JWT claims | `https://veoveo.ai/claims/<kebab-name>` | `https://veoveo.ai/claims/invocation` |
| RFC 9396 authorization detail types | `https://veoveo.ai/oauth/<kebab-name>` | `https://veoveo.ai/oauth/permissions` |
| JSON Schema vocabularies | `https://veoveo.ai/schemas/vocab/<kebab-name>` | `https://veoveo.ai/schemas/vocab/naming` |

The naming vocabulary defines one annotation keyword, `namingProfile`. Schemas keep the
standard 2020-12 meta-schema, under which an unknown keyword is collected as an
annotation, and the vocabulary's meta-schema in `veoveo-types` documents the keyword.

Where a standard already defines a member for a fact, Veoveo uses it. RFC 7591
`software_id` carries a server's slug and `software_version` its version.

## Formats And Schemas

A document's format is identified by one `format` member whose value is an unversioned
URI, `https://veoveo.ai/formats/<kebab-name>`. The format's JSON Schema has the same URI
as its `$id`. Hash domain separators use the format URI followed by a NUL byte, and a
stored marker that binds a hash to its format holds the same URI. A contract that
servers declare compliance with is identified by `https://veoveo.ai/contracts/<kebab-name>`.

Identifiers carry no version. Format and contract URIs, schema `$id`s, extension
identifiers, claim and parameter names, and HTTP paths have no `v1` or `/v2` segment. A
format changes compatibly or takes a new name. Releases carry versions: Git tags, crate
versions, image tags and RFC 7591 `software_version`.

## Domain Names

A concept has one noun across every surface, and each surface spells it in its own form:

| Surface | Form | Time's event |
|---|---|---|
| Resource URI | `<slug>://<plural>/{id}` | `time://events/{event_id}` |
| Resource type | `<slug>.<singular>` | `time.event` |
| Permission | `<slug>.<plural>.<verb>` | `time.events.write` |
| Table | `<schema>.<singular>` | `time.event` |
| Database function | `<schema>.<name>()` | `time.due_events()` |

A server's URI scheme equals its slug. RFC 7595 §3.8 recommends reverse-domain names
for private schemes; Veoveo uses the shorter slug because the Gateway pins each scheme
to one registered server within an installation, which rules out collisions there. The
scheme `veoveo` is reserved for platform resources, such as `veoveo://tasks/{task_id}`,
and no server may register it or a scheme registered with IANA. Resource templates use
RFC 6570 simple expansion.

## Authorization Names

An OAuth scope is coarse and names one namespace, such as `time`. Permissions carry
fine-grained access and follow the permission form above, with a lowercase verb such as
`read`, `write`, `expand` or `administer`. A permission declares one access kind, `read` or `write`, and any
of the risks `sensitive`, `destructive`, `billed`, `physical` and `administer`.

## Errors

| Interface | Error format |
|---|---|
| HTTP APIs | RFC 9457 `application/problem+json` with `type` `https://veoveo.ai/problems/<kebab-code>` |
| OAuth endpoints | The error format of the defining RFC; a 401 carries `WWW-Authenticate: Bearer` |

## Tokens And Keys

An access token's `iss` is the Gateway's issuer URL and its `aud` is the target
server's resource URL. `client_id` names the caller's OAuth client, and an `act` claim
names the Gateway when it acts for a caller. Every key identifier is the key's RFC 7638
thumbprint. Veoveo uses no unregistered URN namespace.

## Identifiers, Times And Digests

Every generated identifier, including `jti`, is a UUIDv7. W3C Trace Context trace and
span identifiers follow that specification instead. JSON timestamps use RFC 3339;
OAuth and JWT members use NumericDate as their specifications require. Every digest is
written `sha256:<lowercase hex>`.

## HTTP Paths

Health endpoints are `/livez` and `/readyz`. Well-known URIs follow RFC 8615 and the
registrations of RFC 8414 and RFC 9728. Administrative routes live under one `/admin/`
prefix.

Issuers, endpoints and resource URLs use `https`. A local installation, whose traffic
stays on one host, may use plain `http` for loopback names (`localhost` and
`*.localhost`, per RFC 6761), loopback addresses and names under `.internal`, the
top-level domain ICANN reserves for private networks. The local installation names its
containers there, such as `vv-gateway.internal`. Every other host requires `https`.

## Environment Variables And Flags

An environment variable is `VEOVEO_<NAME>` in full words, and its command-line flag is
the same name in `kebab-case` without the prefix. Encoded values end in their encoding,
such as `_B64`, and durations in their unit, such as `_SECONDS`.

## Database Names

The kernel owns the schemas `kernel`, `identity`, `tasks` and `audit`, and no server may
register one of them, `public`, `information-schema` or a slug starting with
`pg-` as its slug. A registered server owns the schema named after its slug with each
`-` written as `_`, such as `time_zones` for `time-zones`. Its tables are
`<schema>.<singular>` and its functions `<schema>.<name>()`. Its group role is
`server_<schema>` and the login of each registration `<schema>_<suffix>`. Each schema
notifies changes on the channel named after it. A kernel table whose rows each belong to
one server notifies on `<schema>.<server>` instead, such as `tasks.time`. A server's
migrations record their history in `<schema>._sqlx_migrations`. Columns, constraints and
indexes are `snake_case`.

Every table's comment states its retention: it starts with `Retention:` and says how long
a row stays and what removes it, such as `COMMENT ON TABLE time.event IS 'Retention: 30
days after the event ends; the event pruner deletes it.'`. A query that a
[server schema rule](DATABASE.md#server-schema-rules) exempts says why in a SQL comment
of the form `-- veoveo: <kind> (<reason>)`, where the kind is `all tenants`, `unbounded`
or `scan` and the reason is not empty.

## Runtime And Repository Names

A repository's kind says what it holds, and its code shortens runtime names.
`veoveo-core` and organization singletons such as `veoveo-docs` have no kind.

| Kind | Code | Holds |
|---|---|---|
| `service` | `srv` | A platform process that is not an MCP server, such as the Gateway |
| `mcp` | `mcp` | An MCP server, which publishes its contract through its crate's `contract` feature |
| `runtime` | `rt` | A native or GPU runtime that a service uses |
| `app` | `app` | A user-facing application |
| `sdk` | `sdk` | A library for building Veoveo servers, clients and apps in one language |
| `template` | `tpl` | A GitHub template repository |
| `showcase` | `demo` | A demonstration workload |
| `qualify` | `qual` | A qualification suite for a third-party runtime |

| Thing | Form |
|---|---|
| Repository, crate, binary, image name | `veoveo-<kind>-<name>` |
| Compose project, service and container | `vv-<code>-<name>` with the repository's kind code, or `vv-<name>` for a name without a kind, such as `vv-core` or `vv-postgres`; on a local installation's network, the same name under `.internal`, such as `vv-postgres.internal` |
| Throwaway test container | `vv-qual-<run>-<role>` |
| Environment image tag | `:dev` |
| OpenTelemetry `service.name` | the repository name |
| Image labels | `org.opencontainers.image.title`, `source`, `revision`, `version` and `licenses` |
| Protocol Buffers package | `veoveo.<name>` |
