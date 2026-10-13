# Audit Log

`veoveo-audit` stores the installation's audit records in PostgreSQL, seals them into
signed blocks, exports the blocks and verifies them. The [Audit contract](contract/DESIGN.md)
owns the record, class, detail, target and reader types and the transactional append. The
[database design](../../docs/DATABASE.md) is normative for schemas, notifications,
singletons and query style.

## Standards And Protocols

| Standard or interface | Profile |
|---|---|
| PostgreSQL 18 | Schema `audit`, created by migration `0500_audit.sql` in `platform/store/migrations`, with the read indexes of `0501_audit_record_filters.sql` |
| `xid8` and `pg_snapshot_xmin` | Records are sealed in transaction order below the snapshot horizon |
| `LISTEN` / `NOTIFY` | `kernel.notify_change` on the `audit` channel; records carry their identity as the key, blocks their partition; the tenant member is always null |
| Advisory locks | The sealer holds the `audit.sealer` singleton through `veoveo-platform-store` |
| SQLSTATE | `VV501` reused identity, `VV503` block or intent disagreement, `VV504` recorded export refusal |
| RFC 9162, RFC 8785 and RFC 8032 | Merkle roots over canonical record bytes, JCS block heads and Ed25519 signatures |
| RFC 9562, W3C Trace Context, RFC 3339 | Record and request identities, trace and span identities, timestamps |
| `https://veoveo.ai/formats/audit-record` | The record a producer signs off on, stored as the draft document |
| `https://veoveo.ai/formats/audit-block` | Signed block heads and the signature domain separator |
| `https://veoveo.ai/formats/audit-key-transition-qualification` | The counts the key-transition test prints; it carries no signing material |

Export protocols are in the [export design](src/export/DESIGN.md).

## Storage

| Relation | Holds |
|---|---|
| `audit.detail_type` | Each declared class with its owner, the JSON Schema of its detail and the time of its last declaration |
| `audit.record` | One row per record: the draft as `jsonb` and its occurrence time, generated lookup columns, the insertion sequence, the writing transaction and the database time of the insert |
| `audit.sealer` | The sealed position: the transaction and sequence of the last sealed record |
| `audit.block` | Each signed block document with its partition, sequence, head hash and sealing time |
| `audit.partition_head` | Each partition's newest block sequence, last position and head hash |
| `audit.retention_anchor` | The last block retention removed from each partition |
| `audit.export_delivery`, `audit.export_cursor` | Delivery intents, receipts and refusals per block and destination, and each destination's newest acknowledged block |
| `audit.daily` | A view of record counts per partition, UTC day, class and outcome |

A producer supplies a record's draft and occurrence time. The identity, partition,
class, outcome, actor and trace columns are generated from the draft, so they cannot
disagree with it, and the class column references `audit.detail_type`, so a record of an
undeclared class cannot be stored. A trigger assigns the writing transaction and
`clock_timestamp()` on every insert, and records and blocks reject `UPDATE`. Readers
decode each draft through the audit registry and reject a row whose occurrence time
disagrees with it.

`AuditStore::declare` records the registry's declarations, the platform's included.
Every owner declares before its first append; repeating a declaration changes nothing,
and a changed schema replaces the stored one. `AuditStore::declarations` returns every
stored declaration, from which a reader builds a registry that reads every owner's
records.

`tests/database_conformance.rs` holds the audit queries and the `audit` schema to the
[server schema rules](../../docs/DATABASE.md#server-schema-rules), with each table's
`partition` in place of a tenant column. The sealer, the exporter and retention read
every partition, and their statements say so in their markers.

## Appends

Domain code appends inside its own transaction with `AuditTransactionWrite`, so the
records commit or roll back with the domain change. `AuditStore::append` runs the same
append in a transaction of its own. Repeating an append of the same drafts changes
nothing. A draft that reuses a stored identity with other content raises `VV501`, which
aborts the transaction.

`AuditWriter` serves request handlers. It groups up to 64 records in one transaction
and acknowledges each caller after the commit. A caller waits at most one second for
queue space. Completion records follow an effect that already happened,
so the writer retries them with backoff instead of failing the caller. Shutdown closes
admission and drains the queues within the caller's deadline.

## Sealing

Exactly one replica seals: the holder of the `audit.sealer` singleton. Other replicas
wait on the lock as healthy standbys and take over when the holder's connection ends.
Every seal write, export receipt and retention change runs on the connection that holds
the lock, so a sealer that lost the role cannot write.

A pass reads up to 4,096 records whose `(txid, sequence)` follows the sealed position
and whose transaction ID is below `pg_snapshot_xmin(pg_current_snapshot())`, in that
order. Every transaction below the horizon has ended, so no record can later appear
before the last one read. The pass reads the sealed position once, so the comparison
seeks `record_commit_order` past the sealed records. The sealer signs one block per
partition. Each record takes
the partition position after the one before it, starting at 1, so positions have no
gaps across blocks. One transaction inserts the blocks, advances each partition head
from its previous sequence and hash, and moves the sealed position from the value the
pass read.

A record whose transaction began writing after an open write transaction waits until
that transaction ends; the sealer then retries every 250 milliseconds. After the first
record of a quiet period commits, the sealer waits one second so records committed
together share a block. It pings its role connection every 10 seconds and reports
`Active` once it has caught up. Shutdown seals every committed record before the role is
released; a failed drain is reported.

The verifier compares each record's identity time and occurrence time with its
`recorded_at`, the database time of the insert, and reports records outside the allowed
skew as backdated or future. A sealer that runs hours later does not change that
comparison. A role that can disable the audit triggers can also forge `recorded_at`
before a record is sealed.

## Change Signals

Each record insert notifies with the record's identity, and each block insert or delete
with its partition. `AuditStore::live` reads the signalled records of its partition,
batching signals that arrive together into one query, and reports retention of a
partition block. `blocks_live` wakes when a block of its partition is sealed.
`wait_sealed` returns once a record is at or below the sealed position. Every stream
subscribes before it returns, and a listener reconnection yields a reconcile signal, so
a reader that rereads at that point misses nothing.

## Reads

Every read checks the caller's `AuditReadScope` and the audit registry before SQL, and
selects one partition. Pages follow record identity order with a keyset cursor; every
filter, including whole-target equality, runs in SQL before the limit. A page runs in a
transaction that sets `plan_cache_mode` to `force_custom_plan`, so PostgreSQL plans it
with its actual filters: an actor filter seeks `record_actor`, a trace filter
`record_trace` and a time range may seek `record_occurred`, each ending in the record
identity. The cursor and time bounds are written so that they stay index ranges in a
generic plan as well. Daily counts page through `audit.daily` newest day first over at
most `AUDIT_DAILY_MAX_DAYS`, 366 days, and a later page's days stop at its cursor's day.
A view receipt, an `audit.view` record, admits an audit view only for the same actor
within 15 minutes. Block reads return the members in position order and fail when a member record
is missing. A filtered block read returns every matching member, as an export reads it;
the query's order and page size do not apply, and a query with a cursor is refused.
`export_range` reads the newest head and the last retired block in one statement.

## Retention

Once a minute the sealer retires up to 128 blocks within two seconds. A block is retired
when it is its partition's oldest retained block, was sealed before the cutoff and every
configured export destination acknowledged it. The candidate read walks partition heads
in key order and seeks each oldest block and its deliveries by their primary keys, so
its cost follows the number of partitions. Retirement deletes its records and
delivery intents, makes it the partition's retention anchor and keeps each destination's
cursor.

## Signing-Key Transition

Drain request writers and completion queues, then shut down the service that holds the
sealer role; its shutdown seals the committed backlog. Capture each partition's final
checkpoint in separately protected storage, including its sequence, head hash and last
position. Retain the old public verification key with archived blocks, and keep the
signing seed out of receipts and archive metadata. Remove every process that holds the
old seed, including standbys, before activating replacement-only verification: a
standby with the old seed would sign once it took the role. The replacement sealer
continues the existing sequences, positions and previous-head links.

For complete archive verification, supply the historical and replacement public keys and
require the protected final checkpoint. For post-cutover trust, supply only the
replacement key, start at the protected cutover checkpoint and require the protected
replacement tail. A historical key in the archive key ring also accepts new signatures
by that key, so the two trust profiles use separate verifiers.

`tests/key_transition.rs` qualifies this procedure across a role handoff: it verifies the
retained records and the new suffix, rejects incomplete chains, tampering and unknown
keys, and refuses an old-key block after the cutover under the replacement-only key ring.
It does not prove that an installation removed every holder of the old seed.

## Owner Targets

The installation composes one immutable `AuditTargetRegistry` and gives the same
registry to `AuditStore`, `AuditTransactionWrite` and every reader. Owner codecs are
independent of enabled workloads, so records of a disabled owner stay readable. Target
admission rules are in the [contract design](contract/DESIGN.md).
