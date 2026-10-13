# Audit Record Contract

## Standards And Protocols

| Boundary | Profile |
|---|---|
| RFC 9562 | Canonical UUIDv7 record and HTTP request identities |
| W3C Trace Context | Nonzero lowercase 32-hex trace IDs and 16-hex span IDs |
| RFC 9162, RFC 8785 and RFC 8032 | Typed block heads with decimal-string block sequences and record positions, Merkle roots and Ed25519 signatures |
| JSON Schema draft 2020-12 through jsonschema 0.51.0 | Owner target and detail schemas: closed objects, local definitions, scalar constraints and arrays. Registration rejects open schemas, unresolved or nonlocal references and undeclared schema mechanics. Validation resolves no remote reference |
| Naming vocabulary keyword `namingProfile` | Source annotation admitted through the foundational parser against the complete owner schema; Audit still enforces closed payloads |
| RFC 3339 and JSON | Typed timestamps, tagged closed enums and checked deserialization |
| `https://veoveo.ai/formats/audit-record` | One record shape across producers and readers |
| `https://veoveo.ai/formats/audit-block` | Signed block heads, members and checkpoints |
| PostgreSQL 18 through sqlx 0.9.0 | The `postgres` feature only: `text` bindings for `AuditClass`, `AuditOutcome`, `AuditExportRejection` and `AuditTraceId`, a `uuid` binding for `AuditRecordId`, and `audit.append` for transactional appends |

This library defines the audit record envelope and the registry that admits each owner's
targets and details. It names no protocol, server or domain: every record's action is a
class its owner declared. It depends only on foundational names. Producers and the
`veoveo-audit` crate depend on it. Without the `postgres` feature it has no database
driver, so a producer that only builds drafts does not link sqlx.

## Records

A draft carries the actor, the invocation authority, the request correlation, a target,
a class, a detail, an outcome, a reason, its occurrence time and an optional latency.
The builder derives the partition from the actor's tenant, and accepted outcomes require
the accepted reason. Serialization and input admission share these checks, and the
latency must fit the exact I-JSON integer range. A partition's storage key,
`installation` or `tenant:` followed by the tenant, distinguishes the installation
partition from a tenant literally named `installation`.

The actor names its principal, principal kind, tenant, OAuth client and delegating
principal. The authority names the Work Context, policy revision, scopes and data labels
the call ran with. Attribution that belongs to one component, such as the Gateway's
profile or session family, travels in that component's detail.

A target is a resource by its URI, a server, a Task, a principal, a Work Context, an
audit partition, the installation, or a target an owner registered. The reason
vocabulary covers authorization, authentication, admission and availability outcomes
that any owner can report.

## Classes And Details

An `AuditClass` is `<owner>.<name>`: the owner follows server slug syntax and the name is
a snake_case action, such as `task.update`. An owner declares each class with the JSON
Schema of its detail. `AuditDetailType` ties a Rust detail type to its class, and
`AuditDetailDeclaration::of` generates its declaration. A declaration's schema must be a
closed object in the admitted schema profile.

The platform declares `task.update`, `task.cancel`, `audit.view` and `audit.export`, and
every registry holds them. No other owner may declare a class under `task` or `audit`.
`AuditRegistry::detail` admits a value only when it matches its class's declaration, and
`AuditRegistry::read` returns a recorded detail as its owner's type. Writers check every
detail against the declared schema before they append it. Readers require only that the
class is declared, so records keep reading after their owner's schema changes.

`tests/detail_schema.rs` walks the platform's declared details and rejects free text,
open maps and unreviewed string wrappers. Owners hold their own details to the same
review.

## Reads

The reader's policy owner supplies an `AuditReadScope`. The audit store checks it before
query execution and includes its admitted partition in SQL before decoding or limiting
rows. The public structured `AuditCursor` emits `lastId` and carries its partition and
ordering; query admission rejects a cursor for another partition or ordering, and query
and page readers reject the `last_id` spelling, mixed and duplicate keys. Daily counts
use UTC day bounds and a keyset of day, class and outcome, so readers can traverse every
page without truncating at a fixed total.

The audit domain owns `AuditScope::Read`. Readers check that scope on the current actor.
The actor's tenant selects its tenant partition; the `administrator` and `auditor`
roles also admit installation records.

`AuditRecordSummary` keeps typed targets, classes and details through browser delivery.
`reader_schema(registry)` composes this projection, page queries, daily counts and view
receipts for client generation; its class definition enumerates the registry's classes
and its detail definition admits their declared schemas. A view receipt refers to a
committed `audit.view` record and grants no authority. Each read verifies the current
actor, the selected partition and the receipt's fifteen-minute lifetime. JSON Lines
exports contain a header, records and a completion footer, and readers require that
footer before accepting a complete download.

`AuditContext` carries verified actor, request correlation and invocation authority into
a domain operation, which keeps that attribution for background expiry and terminal
records.

Destination IDs wrap configuration hashes separately from content digests. Export
intents bind both content and signed-block hashes. Closed rejection codes carry no
provider response text and survive replica changes.

## Outcome And Reason Vocabularies

`AuditOutcome` and `AuditReason` use the foundation's `Vocabulary` derive. Their
snake_case spellings, variant order and Serde unit-enum profile form part of the audit
format. `activity/vocabulary_baseline.rs` compares each enum's JSON and schema with its
declaration and checks nonhuman unit-variant ordinals.

## Identity Declaration Mechanics

Audit identities use `Id` with Audit-owned canonical RFC UUIDv7 and lowercase nonzero
hexadecimal admission. Their owner schema functions preserve each UUID pattern,
trace/span length and nonzero constraint. Serde uses checked String conversion for
these IDs. `AuditClass` uses the text form with its own grammar and schema.

## Integrity Counters

`AuditBlockSequence` numbers a partition's blocks and `AuditPosition` its sealed records.
Both admit 1 through i64::MAX, and `AuditPosition::next` yields the following position.
A block head names its first and last position, each member its record and position,
and a checkpoint the partition, sequence, head hash and last position. Positions follow
one another without gaps within a block and from one block to the next. The owner
validator and decimal parser require the canonical unsigned spelling, and Serde
converts through String in both human-readable and binary formats, so JSON and JCS
never round a counter through an IEEE-754 number.

## Owner Admission

`AuditRegistry` holds each owner's target codecs and detail declarations together with
the platform's. `register_target::<T>()` binds a target discriminator to its typed
decoder and closed JSON Schema, and `AuditTargetRegistration<T>` constructs and
retrieves the registered owner type. The immutable registry shares its identity through
clones; a target from another registry cannot enter an append or query. Target equality
compares admitted JSON, and serialization emits the owner object directly without
registry identity or cached payloads.

`AuditDecoder` admits target-bearing drafts, records, queries, summaries, pages and
export lines with an explicit registry. Its JSON parser rejects duplicate object
fields before creating a JSON value. Private input models carry unadmitted values; the
decoder then applies target admission, checks that the class is declared and applies
the draft's own checks. Attribution-only `AuditContext` has no registry state.

Target registration rejects unknown, repeated and platform-colliding discriminators.
A detail declaration that repeats an identical declaration changes nothing; a different
schema for a declared class is refused. Schema composition rejects conflicting
definition names and joins owner branches into the composed unions. An installation
keeps supported read codecs and declarations registered when it disables an owner's
workload; a missing codec or declaration produces a configuration error when reading
existing records. Registering a codec or declaration installs no runtime.

The audit store decodes every stored draft with its configured registry. Every returned
row, including pagination's extra row, validates its record identity, partition, class
and occurrence time. Queries validate target registry identity before executing SQL and
match the entire target object. Transactional appends receive the same registry.

## Transactional Append

With the `postgres` feature, `AuditTransactionWrite` admits up to 4,096 drafts against
a registry, checking each target and each detail against its declaration, and appends
them on the caller's connection with `audit.append`, so the records commit or roll back
with the caller's own writes. It stores each draft as `jsonb` with its occurrence time
truncated to microseconds, the precision PostgreSQL keeps; `stored_time` names that
truncation for readers. The database generates the record's class from its draft and
requires the class to be declared in `audit.detail_type`; an undeclared class fails with
`AuditAppendError::UndeclaredClass`. Repeating an append of the same drafts changes
nothing. A draft that reuses a stored identity with other content fails with
`AuditAppendError::IdentityConflict`, and the caller's transaction must roll back.
