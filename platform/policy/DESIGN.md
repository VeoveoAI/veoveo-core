# Shared Policy Evaluation

## Standards And Protocols

| Boundary | Supported profile |
|---|---|
| Veoveo gateway control plane | Typed profiles, policy sets, principal attributes, installation roles, grants and overrides, and registered owner sections |
| `ai.veoveo/authorization` | Registered servers' permission declarations, read through `PolicyCatalogView::server_authorization`; defined in [the server registration protocol](../../mcp/registration-extension/DESIGN.md) |
| MCP 2026-07-28 | Canonical method/action names and domain resource ownership; evaluation itself performs no protocol I/O |
| JSON / JSON Schema 2020-12 | `mcp/contract` owns policy decisions and core targets; owner contracts supply registered target codecs. The protocol schema adapter composes their bound declarations |
| SurrealDB 3.3.0 | Caller-owned current revision read in `platform/store`; the evaluator has no database dependency |
| Veoveo session-family authority | Internal read-only projection of the stored refresh family and verified request context; no new token or stored wire format |
| RFC 6570 declaration targets | Template discovery and completion carry the foundational `ResourceTemplateUri`. Policy selectors use their existing restricted lexical matcher, not RFC expansion or set containment. |

The Gateway and background workers need the same policy decision. This library owns
the pure evaluator. It depends on canonical contract types without the gateway's agent
runtime, transport and analytics graph.
The gateway keeps its indexed catalog and delegates through `PolicyCatalogView`.
Other services can use `PolicyCatalog`, which validates and indexes one immutable
control-plane revision before it can be evaluated.

PolicyCatalog requires the composition registry when it validates a revision. The
indexed gateway catalog carries the same binding through PolicyCatalogView. Action
handles and contributed targets must match that registry before authorization.

The shared evaluator owns deny precedence, principal conditions and missing-requirement
diagnostics. Owner adapters supply their target and label checks through these helpers. Evaluation
never loads credentials or makes a network request. An Allow decision alone is not a
current authority lease.

The owning service must authenticate the actor, load current grant/revocation state,
resolve current Work Context membership and select an authoritative control-plane
revision. It must account for that read's latency before issuing a bounded lease.
Caching immutable revisions is compatible with this boundary; renewing from a stale
head or unreachable authority store is not.

`PlatformStore::active_gateway_control_revision` reads the active pointer and that
exact retained revision in one round trip. It distinguishes an absent installation
pointer from a dangling or inconsistent revision. The gateway uses this same reader.
Readers capture monotonic time before the request and charge all read and validation
latency against any authority lease they issue afterwards.

`session::SessionFamilyAuthority` owns the pure browser-family binding decision used
by gateway requests and server-side controls. The caller reads the exact signed family ID
from its authority store and verifies the returned record ID. The projection validates
identity, authorization server, profile, client, Work Context, scopes and family state.
Refresh rotation preserves access within that family. Display metadata has no role in
this decision. Corrupt authority fields fail decoding without exposing their values.
Callers still authenticate the token and enforce its issuer, audience and time bounds.
This helper neither reads a database nor authorizes renewal from a cached family.

## Registered Server Decisions

`decide` splits targets by owner. A registered server's tools, resources, resource
templates, prompts and Tasks are decided by installation roles over that server's
declared permissions. The Gateway's own targets, Artifacts and usage are decided by
policy rules, and a rule cannot name an action on a registered server.

`granted_permissions` is pure. It collects the roles granted to the principal or one of
its groups, in the principal's tenant or in every tenant. A held role grants a declared
permission when it allows the permission's access kind and every one of its risks,
after installation overrides add risks or deny the permission. A risk no held role
names is never granted. A target the declaration omits is denied, and server-wide
operations and Tasks require at least one granted permission in the server's
namespace. An allowed decision records the required permission and the full granted
set, which the Gateway forwards as the RFC 9396 authorization detail of the RFC 9068
access token defined by the
[server registration protocol](../../mcp/registration-extension/DESIGN.md#forwarded-identity).

## Internal Client Authority Port

`internal_clients` declares an object-safe current-authority resolver and typed
membership/tool result. The pure evaluator keeps no Store or Agent dependency.
The static implementation evaluates installed registration against the current
profile and rejects managed attribution. A composition selects it only when the
admitted plan excludes Agents. Agent adapters check live registration and collision
before selecting static or managed authority and declare the owner tables that wake
subscribers. Callers authenticate assertions, validate identity/control freshness,
and repeat resolution before delivery. Resolver results grant no cached lease.
