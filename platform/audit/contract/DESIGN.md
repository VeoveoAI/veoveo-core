# Audit Record Contract

## Standards And Protocols

| Boundary | Profile |
|---|---|
| NIST SP 800-53 Rev. 5 AU-3 | Every record carries what type of event occurred (its class), when (its occurrence time), where (its source server and request), the source (request, trace and peer), the outcome, and the identity of the actor, the acting server and the target. AU-3(1) additional information travels in the owner's detail, and AU-3(3) limits it: details never hold payloads, tokens or secrets |
| NIST SP 800-53 Rev. 5 AU-9(3) and AU-10 | Records are sealed into Ed25519-signed blocks over a hash chain in `veoveo-audit` |
| [OCSF 1.9.0](https://schema.ocsf.io/1.9.0/) | Field mapping of the envelope: class to `class_uid` and `activity_id` as each owner declares them, outcome to `status_id` and the Security Control profile's `action_id`, reason to `status_code`, acting server to `actor.application`, target to `resources`, tenant to `metadata.tenant_uid`, request to `metadata.correlation_uid` and the sealed position to `metadata.sequence`; Work Context, acting Task and details travel in an OCSF extension |
| RFC 3986 | Targets: absolute URIs without a fragment, at most 2048 bytes; a Task is its RFC 9562 UUID URN |
| RFC 9562 | Canonical UUIDv7 record and HTTP request identities |
| W3C Trace Context | Nonzero lowercase 32-hex trace IDs and 16-hex span IDs |
| RFC 9162, RFC 8785 and RFC 8032 | Typed block heads with decimal-string block sequences and record positions, Merkle roots and Ed25519 signatures |
| JSON Schema draft 2020-12 | Owner detail schemas, stored with their class as documentation for readers; writers check none |
| RFC 3339 and JSON | Typed timestamps, tagged closed enums and checked deserialization |
| `https://veoveo.ai/formats/audit-record` | One record shape across producers and readers |
| `https://veoveo.ai/formats/audit-block` | Signed block heads, members and checkpoints |
| PostgreSQL 18 through sqlx 0.9.0 | The `postgres` feature only: `text` bindings for `AuditClass`, `AuditOutcome`, `AuditReason`, `AuditTarget`, `AuditExportRejection` and `AuditTraceId`, a `uuid` binding for `AuditRecordId`, and `audit.append` for transactional appends |

This library defines the audit record envelope and the registry of owner-declared
classes. It names no protocol, server or domain: every record's action is a class its
owner declared and every reason a code its owner chose. It depends only on foundational
names. Producers and the `veoveo-audit` crate depend on it. Without the `postgres`
feature it has no database driver, so a producer that only builds drafts does not link
sqlx.

## Records

| Field | Set by | Rule |
|---|---|---|
| `id`, `occurred_at` | The producer | A UUIDv7 and the time the action happened; the database adds `recorded_at` |
| `partition` | The builder | The actor's tenant, or the installation for a record without one |
| `class` | The owner | `<owner>.<name>`, declared; the database derives the record's source from its owner |
| `outcome` | The owner | `success`, `denied`, `failure` or `unknown` |
| `reason` | The owner | Optional; a code the owner defines, scoped to the record's source |
| `actor` | The owner | Principal, principal kind, tenant and Work Context, or none for the system |
| `via` | The owner, from the SDK | The server that acted for the actor through the Hub, and its Task |
| `target` | The owner | Optional; an absolute URI in the owner's address form, such as `artifact://artifacts/{id}`, or `task_target(id)` for a Task |
| `request` | The owner, from the SDK | Request ID, trace and span IDs, peer address and the Task the action ran in |
| `detail` | The owner | A JSON object of at most 16 KiB of compact JSON, the owner's own type |

`AuditDraft::builder(request, detail, outcome)` starts a record, and the builder adds
the actor, acting server, target, reason, identity and occurrence time. Serialization and
input admission share the envelope checks: the partition follows the actor's tenant and
the detail fits its limit. A partition's storage key, `installation` or `tenant:`
followed by the tenant, distinguishes the installation partition from a tenant literally
named `installation`. Attribution that belongs to one component, such as the Hub's
OAuth client, profile or policy revision, travels in that component's detail.

## Classes And Details

An `AuditClass` is `<owner>.<name>`: the owner follows server slug syntax and the name is
a snake_case action, such as `task.update`. `AuditDetailType` ties a Rust detail type to
its class and to the `AuditOcsfClass` it maps to, API Activity with activity Other unless
the owner names another. `AuditDetailDeclaration::of` generates the declaration with the
type's JSON Schema, which readers receive as documentation.

The platform declares `task.update`, `task.cancel`, `audit.view` and `audit.export`, and
every registry holds them. No other owner may declare a class under `task` or `audit`.
`AuditRegistry::detail` admits a value of a declared class whose JSON is an object of at
most 16 KiB, and `AuditRegistry::read` returns a recorded detail as its owner's type.
Writers check no schema: the owner's types keep a detail's shape, and readers require
only that the class is declared, so records keep reading after their owner's types
change. A declaration that repeats an identical one changes nothing; a different
declaration of a declared class is refused.

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

`AuditRecordSummary` keeps typed targets, acting servers, classes, reasons and details
through browser delivery. `reader_schema(registry)` composes this projection, page
queries, daily counts and view receipts for client generation; its class definition
enumerates the registry's classes and its detail definition admits their documented
schemas. A view receipt refers to a
committed `audit.view` record, whose detail names the partition it views, and grants no
authority. Each read verifies the current
actor, the selected partition and the receipt's fifteen-minute lifetime. JSON Lines
exports contain a header, records and a completion footer, and readers require that
footer before accepting a complete download.

`AuditContext` carries verified actor, request correlation and invocation authority into
a domain operation, which keeps that attribution for background expiry and terminal
records.

Destination IDs wrap configuration hashes separately from content digests. Export
intents bind both content and signed-block hashes. Closed rejection codes carry no
provider response text and survive replica changes.

## Outcome And Reason

`AuditOutcome` uses the foundation's `Vocabulary` derive; its snake_case spellings,
variant order and Serde unit-enum profile form part of the audit format, and
`activity/vocabulary_baseline.rs` compares its JSON and schema with its declaration.
`AuditReason` is an identity of the `text` form: a lowercase letter followed by at most
63 lowercase letters, digits and underscores, which each owner defines for its own
records.

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

`AuditRegistry` holds each owner's class declarations together with the platform's.
`AuditDecoder` admits drafts, records, queries, summaries, pages and export lines with an
explicit registry. Its JSON parser rejects duplicate object fields before creating a
JSON value. Private input models carry unadmitted values; the decoder then checks that
the class is declared and applies the draft's own checks. Attribution-only
`AuditContext` has no registry state. An installation keeps declarations registered
when it disables an owner's workload; a missing declaration produces a configuration
error when reading existing records.

## Stored Records

`AuditStoredRecord` is a record as the audit log stores it: the draft document exactly
as its producer appended it and the database's recorded time. `AuditStoredRecord::admit`
checks the envelope with the same rules as a draft and requires the detail to be an
object, without decoding it. Its
serialization is the stored document, so its canonical bytes equal those of the decoded
`AuditRecord`, and `AuditRecord::stored` converts one into the other.
`AuditSealedRecord` names what sealing and verification read from a record, and both
types implement it.

## Transactional Append

With the `postgres` feature, `AuditTransactionWrite` admits up to 4,096 drafts against
a registry, checking that each class is declared and each detail fits its limit, and
appends
them on the caller's connection with `audit.append`, so the records commit or roll back
with the caller's own writes. It stores each draft as `jsonb` with its occurrence time
truncated to microseconds, the precision PostgreSQL keeps; `stored_time` names that
truncation for readers. The database generates the record's class from its draft and
requires the class to be declared in `audit.detail_type`; an undeclared class fails with
`AuditAppendError::UndeclaredClass`. Repeating an append of the same drafts changes
nothing. A draft that reuses a stored identity with other content fails with
`AuditAppendError::IdentityConflict`, and the caller's transaction must roll back.
