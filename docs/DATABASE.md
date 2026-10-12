# Database

Every Veoveo component stores its state in one PostgreSQL database per installation.
Core owns the kernel schemas, the connection layer, process instances and change
notifications. A server owns one schema of its own and reaches kernel data through
grants that Postgres enforces.

## Standards And Protocols

| Standard or interface | Profile |
|---|---|
| [PostgreSQL 18](https://www.postgresql.org/docs/18/) | Server 18.6, the latest stable release. Installations use the official `postgres` image pinned by digest; tests run the same major version from the PGDG packages |
| PostgreSQL frontend/backend protocol | Through [sqlx](https://docs.rs/sqlx/0.9.0) 0.9.0 with Rustls; every static query is checked at compile time against the migrated schema |
| SQL migrations | sqlx `Migrator`, one migration set per owner schema, checksummed and applied under the migrator's advisory lock |
| `LISTEN` / `NOTIFY` | Change notifications; payloads are JSON objects under 8,000 bytes |
| Advisory locks | Process instance liveness and singleton election |
| `SELECT … FOR UPDATE SKIP LOCKED` | Work claims |
| Row-level security, roles and `GRANT` | Isolation between the kernel and registered servers |
| `xid8` and `pg_snapshot_xmin` | Commit-safe ordering for the audit log |

## Database, Schemas And Roles

An installation has one database. Each owner has one schema:

| Schema | Owner | Contents |
|---|---|---|
| `kernel` | Core | Migration history, process instances and shared functions |
| `identity` | Core | Enterprises, tenants, principals and Work Contexts |
| `gateway` | Core | Control-plane revisions, OAuth state, registrations and Task routes |
| `tasks` | Core | Tasks, their input exchanges and idempotency keys |
| `audit` | Core | Audit records, blocks, seals, export cursors and indexing windows |
| `<server>` | A registered server | The server's own tables and functions; the name is the server slug with `-` replaced by `_` |

The Gateway connects as the installation owner, a role that owns the database and has
`CREATEROLE`. It applies the kernel migrations at startup.

Registration gives each server three roles:

- `server_<server>` is a group role without login. It owns the server's schema and is a
  member of `veoveo_server`, the kernel's group role for every server.
- `<server>_<suffix>` is a login role created for one registration, a member of
  `server_<server>` with `SET` and `INHERIT`. The registration grant carries its name and a
  random password. Deregistration drops it.
- `veoveo_server` holds the kernel grants that every server needs. The kernel migrations
  define it.

A server runs its own migrations at startup after `SET ROLE server_<server>`, so the
group role owns every object and a later registration of the same server keeps access.

Kernel tables that servers share enable row-level security. A policy admits a row when
its `server` column names a server whose group role the current user belongs to.
`kernel.current_servers()` returns those slugs. Kernel writes that span owners, such as
recording an identity, are `SECURITY DEFINER` functions granted to `veoveo_server`.

## Connections

A component reads `VEOVEO_DATABASE_URL` (a `postgres://` URL without a password) and
`VEOVEO_DATABASE_PASSWORD`. A registered server receives both in its registration grant
instead. Each process keeps one pool for queries, one dedicated connection for its
instance lock and one `PgListener` connection for notifications.

## Migrations

Kernel migrations live in `platform/store/migrations` and record their history in
`kernel._sqlx_migrations`. A server's migrations live in its own crate and record their
history in `<server>._sqlx_migrations`. Each file is plain SQL, applied in version order
inside one transaction. A changed checksum stops startup. Concurrent replicas wait on the
migrator's advisory lock, so only one applies a given version.

## Process Instances

Every process that owns work starts an instance. It inserts a row into `kernel.instance`
on a dedicated connection and takes a session advisory lock keyed by that row's ID. The
lock lives as long as the connection. `kernel.instance_alive(id)` reports whether that
lock is held, using `pg_locks`. A process that exits or loses its connection releases the
lock at once; a host that disappears releases it when TCP keepalive closes the
connection. A clean shutdown deletes the row.

Work that a process owns references its instance through `ON DELETE SET NULL`. Each live
process periodically calls `kernel.reap_instances()`, which deletes the rows of instances
whose locks are free. The cascade clears ownership in the same transaction, and a trigger
moves the released work to its recovery state. Settlement writes compare the owner
instance, so a process whose instance was reaped cannot settle work it no longer owns.

Connection poolers that multiplex sessions, such as PgBouncer in transaction mode, break
session advisory locks. Installations connect components to Postgres directly.

## Singletons

A role that exactly one replica performs, such as the audit sealer, takes a session
advisory lock on a dedicated connection and performs its writes on that connection.
Standby replicas wait on the same lock. When the holder's connection ends, Postgres grants
the lock to the next waiter.

## Work Claims

Workers claim queued work with `SELECT … FOR UPDATE SKIP LOCKED` and record their instance
as the owner in the same transaction. A trigger notifies the owner's channel when work is
created or released, and workers also claim at startup and after each reap.

## Change Notifications

Triggers call `pg_notify` with a JSON payload naming the table, the operation and the
row's key and tenant. Each schema has one channel named after the schema. A consumer
listens through `PgListener`. When the listener reports a lost connection, the consumer
rereads the current state of everything it watches, because Postgres does not replay
notifications sent while it was disconnected. Notifications are wake signals: consumers
read current rows under their own authorization and never trust payload content as data.

## Timed Changes

A row that becomes due at a known time stores that time in an indexed column. Each
replica reads the earliest pending time, sleeps until it or until a notification on its
channel, then applies the change with a conditional `UPDATE … WHERE` that matches only
pending rows. Replicas racing for the same row change it once.

## Audit Ordering

Each audit record stores the `xid8` of the transaction that wrote it. The sealer reads
records whose transaction ID is below `pg_snapshot_xmin(pg_current_snapshot())`, ordered
by transaction ID and then by insertion sequence. No transaction that could still commit
sorts before that horizon, so sealing never skips a record. The sealer assigns each sealed
record its gapless position in the partition.

## Typed Queries

Each query lives in its own `.sql` file beside the code that runs it and goes through
sqlx's `query_file_as!` or `query_file!` macros. The compiler checks the statement against
the migrated schema: table and column names, column types, nullability and parameter
types. A column that decodes into a domain type names it with sqlx's override syntax,
`status AS "status: TaskStatus"`. Builds without a database use the metadata in the
crate's `.sqlx` directory with `SQLX_OFFLINE=true`. `cargo sqlx prepare` regenerates that
metadata against a migrated database, and the check stage fails when it is stale.

Statements whose text depends on runtime values, such as creating a registration's
login role, live in PL/pgSQL functions that quote identifiers and literals with
`format('%I', …)` and `format('%L', …)`. Rust calls those functions through checked
queries; no Rust code concatenates SQL.

## Column Types

| Value | Column type |
|---|---|
| Identifiers | `uuid` |
| Closed vocabularies | `text` with a `CHECK` constraint listing the wire spellings |
| Timestamps | `timestamptz` |
| Counters and sizes | `bigint` with a `CHECK` for the admitted range |
| Open JSON payloads | `jsonb` |
| Digests | `text` in `sha256:<hex>` form |

Rust types bind through `sqlx::Type`. The `Vocabulary` derive and the `id` attribute
generate the bindings, so domain types stay typed until the driver boundary.

## Testing

`veoveo-test-fixtures` starts one Postgres server per test binary from the `initdb` and
`postgres` executables on `PATH`, listening on a loopback port with a private socket
directory. It migrates a template database once; each `TestDb` clones a fresh database
from that template and drops it when the test ends. The development image installs the
PGDG `postgresql-18` packages and puts their binaries on `PATH`.
