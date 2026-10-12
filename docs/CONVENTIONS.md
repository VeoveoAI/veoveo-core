# Naming And Format Conventions

Every name, identifier, format, wire field and protocol element in a Veoveo repository
follows a published standard. Where no standard applies, it follows a rule in this
document. An extension uses the extension mechanism and naming style of the standard
it extends. A name or format that fits neither is a defect.

## Standards And Protocols

| Standard | Governs |
|---|---|
| [MCP](https://modelcontextprotocol.io/specification/) `2026-07-28` and its extension rules | MCP messages, tool names, `_meta` keys and `capabilities.extensions` identifiers |
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
| MCP messages, Veoveo HTTP API bodies, configuration files and Veoveo formats | `camelCase` |
| OAuth, OpenID Connect and JWT documents, including objects nested in a Veoveo claim | `snake_case`, as their specifications use |
| SurrealDB records, and formats whose canonical bytes are a stored record, such as Audit records and blocks | `snake_case`, the database's naming |

Enum values are `snake_case` in every family. One JSON object uses one casing; an
extension nested inside another family's document follows that document's casing. A
stored record is not returned as an API body; the API defines its own camelCase type.

## Extension Names

| Host standard | Form | Example |
|---|---|---|
| MCP extension identifiers and `_meta` keys | `ai.veoveo/<kebab-name>` | `ai.veoveo/knowledge-source` |
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
servers declare compliance with is identified by `https://veoveo.ai/contracts/<kebab-name>`,
such as `https://veoveo.ai/contracts/hosted-mcp`.

Nothing carries a version number before it is released. After release, a format changes
compatibly or takes a new name; the release process records that policy.

## Domain Names

A concept has one noun across every surface, and each surface spells it in its own form:

| Surface | Form | Time's event |
|---|---|---|
| MCP tool | `<verb>_<noun>` in `snake_case` | `create_event`, `cancel_event` |
| Resource URI | `<slug>://<plural>/{id}` | `time://events/{event_id}` |
| Resource type | `<slug>.<singular>` | `time.event` |
| Permission | `<slug>.<plural>.<verb>` | `time.events.write` |
| Knowledge collection | `<slug>.<plural>` | `time.events` |
| Table | `<slug>_<singular>` | `time_event` |
| Database function | `fn::<slug>::<name>` | `fn::time::due_events` |

A server's URI scheme equals its slug. RFC 7595 §3.8 recommends reverse-domain names
for private schemes; Veoveo uses the shorter slug because the Gateway pins each scheme
to one registered server within an installation, which rules out collisions there. The
scheme `veoveo` is reserved for platform resources, such as `veoveo://tasks/{task_id}`,
and no server may register it or a scheme registered with IANA. The Gateway projects a
tool as `<slug>__<tool>` when it lists tools from several servers. Resource templates use
RFC 6570 simple expansion, and lists page through MCP cursors.

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
| JSON-RPC | MCP's codes, plus Veoveo codes from `-31999` to `-31000`, outside the range JSON-RPC 2.0 reserves, listed in the `veoveo-mcp-contract` code registry |

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
never leaves one host, may use plain `http` for loopback names (`localhost` and
`*.localhost`, per RFC 6761), loopback addresses and OpenShell's host alias
`host.openshell.internal`, which reaches the host's loopback interface. MCP's
authorization specification makes the same exception for localhost. Every other host
requires `https`.

## Environment Variables And Flags

An environment variable is `VEOVEO_<NAME>` in full words, and its command-line flag is
the same name in `kebab-case` without the prefix. Encoded values end in their encoding,
such as `_B64`, and durations in their unit, such as `_SECONDS`.

## Database Names

Kernel tables are reserved and unprefixed. Kernel SQL APIs that other modules call live
under `fn::kernel::<module>::`, and a kernel module's private functions under
`fn::<module>::`. Kernel module names are reserved, and no server may register one as its
slug. Every other module prefixes its tables with `<slug>_` and its functions with
`fn::<slug>::`.

## Runtime And Repository Names

| Thing | Form |
|---|---|
| Repository, crate, binary, image name | `veoveo-<kind>-<name>` |
| OpenShell sandbox and provider, Compose project, service and container | `vv-<code>-<name>` with the repository's kind code, or `vv-<name>` for a name without a kind, such as `vv-core` or `vv-surrealdb`; at most 19 characters |
| Throwaway test sandbox | `vv-qual-<run>-<role>` |
| Environment image tag | `:dev` |
| OpenTelemetry `service.name` | the repository name |
| Image labels | `org.opencontainers.image.title`, `source`, `revision`, `version` and `licenses` |
| Protocol Buffers package | `veoveo.<name>` |
