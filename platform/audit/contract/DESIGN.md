# Audit Record Contract

## Standards And Protocols

| Boundary | Profile |
|---|---|
| RFC 9562 | Canonical UUIDv7 record and HTTP request identities |
| W3C Trace Context | Nonzero lowercase 32-hex trace IDs and 16-hex span IDs |
| RFC 9162, RFC 8785 and RFC 8032 | Typed block heads with decimal-string block sequences and record positions, Merkle roots and Ed25519 signatures |
| JSON Schema draft 2020-12 | Composed closed target union, local definitions, scalar constraints and arrays; owner registration rejects open schemas, unresolved or nonlocal references and undeclared schema mechanics |
| Naming vocabulary keyword `namingProfile` | Source annotation admitted through the foundational parser against the complete owner schema; Audit still enforces closed target payloads |
| RFC 3339 and JSON | Typed timestamps, tagged closed enums and checked deserialization |
| `https://veoveo.ai/formats/audit-record` | One reviewed record shape across platform producers and readers |
| `https://veoveo.ai/formats/audit-block` | Signed block heads, members and checkpoints |
| `ai.veoveo/knowledge-source` | Reviewed observation fields from the protocol-independent extension contract |
| PostgreSQL 18 through sqlx 0.9.0 | The `postgres` feature only: `text` bindings for `AuditClass`, `AuditOutcome`, `AuditExportRejection` and `AuditTraceId`, a `uuid` binding for `AuditRecordId`, and `audit.append` for transactional appends |

This library owns the protocol-independent audit vocabulary. It depends on foundational
names and lightweight Artifact identities. Computers supplies its Audit target codec
through the installation registry. Producers, MCP adapters and the `veoveo-audit` crate
depend on this crate. Without the `postgres` feature it has no database driver, so a
producer that only builds drafts does not link sqlx.

The draft builder derives the partition from the authenticated actor. Accepted outcomes
require the accepted reason. Serialization and input admission share these checks.
Details expose only reviewed enums, identifiers, digests, counters and timestamps.
`tests/detail_schema.rs` walks the generated detail schema and rejects free text,
open maps and unreviewed string wrappers. It also recognizes the reviewed, inlined
SHA-256 pattern. These identifier allowlists belong to this contract and must be
reviewed when a detail gains a new identifier type.
Numeric counters must fit the exact I-JSON integer range before a draft can be stored.
Tool, discovery and Task details must match the corresponding typed target, and a
registration detail targets the registered server. The Gateway records each namespace
claim, migration, registration, declaration update, removal and lease expiry; lease
renewals produce no record. A platform
Task uses its Task identity; an external Task uses its opaque gateway route and
server identity. These identities have separate variants.
Knowledge read details bind the member and collection owner to their resource target.
Successful reads require an observation whose conditional status matches the outcome.
Denied or failed reads cannot claim a returned revision. `KnowledgeReadObservation`
copies source identity, revision, digest, times, attribution and access fields. Its
external identity excludes navigation URLs because they can carry signed query
credentials. The closed audit schema rejects those URLs on input as well.
An absent or null observation access field decodes to None. The Audit producer
omits None and preserves the frozen access policy spellings for Some; receiving
null does not change the serialized record profile.
The access descriptor includes the source's closed read policy and its optional
`GatewayProfileId` restriction. That identifier is an installation profile route token
validated by the foundational type; it carries no credential or free-form description.
`IndexingRead` admits only collection-matching resource reads by a tenant service
client. Denials cannot enter an aggregate. `IndexingWindow` requires a matching server
target, five-minute UTC bounds, positive reads and consistent outcome counters. Its
occurrence time equals the window end. Constructor and decode checks share these rules.
A partition's storage key, `installation` or `tenant:` followed by the tenant,
distinguishes the installation partition from a tenant literally named `installation`.

The reader's policy owner supplies an `AuditReadScope`. The audit store checks it before
query execution and includes its admitted partition in SQL before decoding or limiting
rows.
The public structured `AuditCursor` emits `lastId` and carries its partition and ordering;
query admission rejects a cursor for another partition or ordering. Query and page readers
reject retired `last_id`, mixed and duplicate keys. Installation upgrades drain active Audit
queries and update Gateway and generated Console readers together; this unversioned query
keyset has no compatibility alias. Signed record, draft, block and checkpoint bytes keep their
frozen profiles. Daily counts
use UTC day bounds and a keyset of day, class and outcome; readers can traverse every
page without silently truncating at a fixed total.

The audit domain owns `AuditScope::Read`. Protocol cores carry its `ScopeName` without
defining the domain vocabulary. Gateway readers check that scope on the current actor.
The actor's tenant selects its tenant partition; the `administrator` and `auditor`
roles also admit installation records.

`AuditRecordSummary` keeps typed targets and activities through browser delivery.
`reader_schema(registry)` composes this projection, page queries, daily counts and view receipts
for the existing client generator. A view receipt refers to a committed access record;
it grants no authority. Each read verifies the current actor, profile, selected
partition and receipt lifetime. JSON Lines exports contain a header, records and a
completion footer. Readers require that footer before accepting a complete download.

`AuditContext` carries verified actor, request correlation and invocation authority into
a domain operation. The MCP adapter derives it from `GatewayRequestContext` only after
validating the source-principal, actor and invocation relationship. Domain services keep
that attribution for background expiry and terminal records. Dictation summaries expose
an end reason, accepted chunk count and audio duration; they cannot carry transcript text.

Implementation and qualification are in progress under the
[audit acceptance](https://github.com/BiomaAI/veoveo/blob/0c33cf8a7e5a1d46f953702553ac6643f99c1363/docs/CONTRACT_CONSISTENCY_PLAN.md#unified-audit-log).

Destination IDs wrap configuration hashes separately from content digests. Export
intents bind both content and signed-block hashes. Closed rejection codes carry no
provider response text and survive replica changes.

## Activity Vocabularies

Activity enums use the foundation's `Vocabulary` derive. Their snake_case spellings,
variant order and Serde unit-enum profile form part of the frozen audit format.
Owner-local tests compare every activity enum's JSON and schema with its previous
declaration and check nonhuman unit-variant ordinals. The derive introduces no audit
variants or authorization decisions.

## Identity Declaration Mechanics

Audit identities use `Id` with Audit-owned canonical RFC UUIDv7 and lowercase nonzero hexadecimal admission. Their owner schema functions preserve each UUID pattern, trace/span length and nonzero constraint. Serde uses checked String conversion for these IDs; the independent Activity enum wire profile does not change.

## Integrity Counters

`AuditBlockSequence` numbers a partition's blocks and `AuditPosition` its sealed records.
Both admit 1 through i64::MAX, and `AuditPosition::next` yields the following position.
A block head names its first and last position, each member its record and position,
and a checkpoint the partition, sequence, head hash and last position. Positions follow
one another without gaps within a block and from one block to the next. The owner
validator and decimal parser require the canonical unsigned spelling, and Serde
converts through String in both human-readable and binary formats, so JSON and JCS
never round a counter through an IEEE-754 number.

## Owner Target Admission

`AuditTargetRegistry` binds each owner discriminator to its typed decoder and closed
JSON Schema. `AuditTargetRegistration<T>` constructs and retrieves the registered owner
type. The immutable registry shares its identity through clones; a target from another
registry cannot enter an append or query. Target equality compares admitted JSON.
Serialization emits the owner object directly and excludes registry identity and cached
payloads.

`AuditDecoder` admits target-bearing drafts, records, queries, summaries, pages and
export lines with an explicit registry. Its JSON parser rejects duplicate object
fields before creating a JSON value. Private input models carry unadmitted values;
the decoder then applies target admission and the draft's existing relationship
checks. Attribution-only `AuditContext` has no registry state.

Owner registration rejects unknown, repeated and core-colliding discriminators.
Schema composition rejects duplicate definition names and joins owner branches into
the target's closed union. An installation keeps supported read codecs registered
when it disables an owner's workload. Missing codecs produce a configuration error
when reading existing records. Registering a codec installs no runtime or database
schema.

The audit store decodes every stored draft with its configured registry. Every returned
row, including pagination's extra row, validates its record identity and partition.
Queries validate target registry identity before executing SQL and match the entire
target object. Transactional appends receive the same registry.

## Transactional Append

With the `postgres` feature, `AuditTransactionWrite` admits up to 4,096 drafts against
a target registry and appends them on the caller's connection with `audit.append`, so
the records commit or roll back with the caller's own writes. It stores each draft as
`jsonb` with its class and its occurrence time truncated to microseconds, the precision
PostgreSQL keeps; `stored_time` names that truncation for readers. Repeating an append
of the same drafts changes nothing. A draft that reuses a stored identity with other
content fails with `AuditAppendError::IdentityConflict`, and the caller's transaction
must roll back.

## Frozen Knowledge Access Encoding

`knowledge/frozen_access.rs` maps the public Knowledge extension's admitted access
descriptor into Audit's original read-policy spellings. Audit serialization and
decoding keep the kebab-case policy values in existing records and hash inputs.
The projection changes no grant, expiry, owner or Work Context relationship and
performs no authorization. Public Knowledge observations use their owner spelling.
