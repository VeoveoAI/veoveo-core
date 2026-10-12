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
| SQL migrations | sqlx `Migrator`, one migration set for the kernel schemas and one per server schema, checksummed and applied under the migrator's advisory lock; `platform/store/sqlx.toml` names the kernel history table |
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

A server name is its slug: a lowercase letter followed by at most 47 lowercase letters,
digits and hyphens. In the role and schema names below, `<server>` is that slug with
each `-` written as `_`; a slug whose schema would be a kernel schema, `public`,
`information_schema` or start with `pg_` is refused. Registration gives each server
three roles:

- `server_<server>` is a group role without login. It owns the server's schema and is a
  member of `veoveo_server`, the kernel's group role for every server.
- `<server>_<suffix>` is a login role created for one registration, a member of
  `server_<server>` with `SET` and `INHERIT`. The registration grant carries its name and a
  random password. Deregistration drops it.
- `veoveo_server` holds the kernel grants that every server needs. The kernel migrations
  define it.

The Gateway calls `kernel.ensure_server(server)`, which creates the group role and the
schema and changes nothing when they exist, `kernel.create_server_login(server, login,
password)` and `kernel.drop_server_login(login)`. Only the installation owner may run
them.

A server runs its own migrations at startup after `SET ROLE server_<server>`, so the
group role owns every object and a later registration of the same server keeps access.

Kernel tables that servers share enable row-level security. A policy admits a row when
its `server` column names a server whose group role the current user belongs to.
`kernel.current_servers()` returns those slugs as a `text[]`; a policy reads it through a
scalar subquery, `server = ANY ((SELECT kernel.current_servers())::text[])`, so it runs
once per statement. Kernel writes that span owners, such as recording an identity, are
`SECURITY DEFINER` functions granted to `veoveo_server`. Every function in a kernel
schema is revoked from `PUBLIC`, and the migrations grant each one that servers call.

## Connections

A component reads `VEOVEO_DATABASE_URL` (a `postgres://` URL without a password) and
`VEOVEO_DATABASE_PASSWORD`. A registered server receives the URL, its login and its
password in its registration grant instead. `StoreConfig` takes the URL, an optional
login that replaces the URL's user, and the password as a secret. Each process keeps one
pool for queries, one dedicated connection for its instance lock and one `PgListener`
connection for notifications.

## Migrations

Kernel migrations live in `platform/store/migrations` and record their history in
`kernel._sqlx_migrations`. Their versions are grouped by schema in hundreds: `01xx` for
`kernel`, `02xx` for `identity`, `03xx` for `gateway`, `04xx` for `tasks` and `05xx` for
`audit`. A server's migrations live in its own crate and record their history in
`<server>._sqlx_migrations`; `PlatformStore::migrate_server` applies them on a dedicated
connection after taking the group role, with the server's schema as the search path.
Each file is plain SQL, applied in version order inside one transaction. A changed
checksum stops startup. Concurrent replicas wait on the migrator's advisory lock, so only
one applies a given version.

## Process Instances

Every process that owns work starts an instance. `kernel.start_instance(server)` inserts
a row into `kernel.instance` on a dedicated connection and takes the session advisory
lock `(1987471726, id)` before the row commits. The row records the owning server, the
login that started it and its start time; a kernel component names itself, such as
`gateway`. The lock lives as long as the connection. `kernel.instance_alive(id)` reports
whether that lock is held, using `pg_locks`. A process that exits or loses its connection
releases the lock at once; a host that disappears releases it when TCP keepalive closes
the connection. A clean shutdown deletes the row.

The `Instance` type checks its connection every 5 seconds and reports itself lost when
a check fails or takes longer than 5 seconds; the process then stops acting on the work
it owned. Work that a process owns references its instance through `ON DELETE SET NULL`.
Each live process calls `kernel.reap_instances()` when it starts and every 15 seconds; the
function deletes the rows of instances whose locks are free. The cascade clears
ownership in the same transaction, and a trigger moves the released work to its recovery
state. Settlement writes compare the owner instance, so a process whose instance was
reaped cannot settle work it no longer owns.

Connection poolers that multiplex sessions, such as PgBouncer in transaction mode, break
session advisory locks. Installations connect components to Postgres directly.

## Singletons

A role that exactly one replica performs, such as the audit sealer, takes a session
advisory lock on a dedicated connection and performs its writes on that connection.
`kernel.lock_singleton(name)` locks `(1987474279, hashtext(name))`, and a name starts with
its owner's slug, such as `audit.sealer`. Standby replicas wait on the same lock. When
the holder's connection ends, Postgres grants the lock to the next waiter.

## Work Claims

Workers claim queued work with `SELECT … FOR UPDATE SKIP LOCKED` and record their instance
as the owner in the same transaction. A trigger notifies the owner's channel when work is
created or released, and workers also claim at startup and after each reap.

## Change Notifications

Row triggers execute `kernel.notify_change(key_column, tenant_column)`, which calls
`pg_notify` with a JSON payload naming the table, the operation (`insert`, `update` or
`delete`) and the row's key and tenant; a key longer than 1,024 bytes is sent as null.
Each schema has one channel named after the schema. A consumer listens through
`PlatformStore::listen`, a `PgListener` on its own connection that yields `Reconcile`
once subscribed and again after every reconnection, because Postgres does not replay
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
record its gapless position in the partition. `audit.sealer` holds the transaction ID and
sequence of the last sealed record, and a record is sealed when its pair is at most that
one. A write transaction that stays open holds back every record whose transaction ID is
higher until it ends.

## Typed Queries

Each query lives in its own `.sql` file beside the code that runs it and goes through
sqlx's `query_file_as!` or `query_file!` macros. The compiler checks the statement against
the migrated schema: table and column names, column types, nullability and parameter
types. A column that decodes into a domain type names it with sqlx's override syntax,
`status AS "status: TaskStatus"`, and a bind parameter of a domain type is written
`status as TaskStatus`. Builds use the metadata in the workspace's `.sqlx` directory;
`.cargo/config.toml` sets `SQLX_OFFLINE=true`. `cargo sqlx prepare --workspace --
--all-targets --all-features` regenerates that metadata against a migrated database, and
the check stage fails when it is stale.

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

Rust types bind through `sqlx::Type`. `#[vocabulary(postgres)]` and the `id`
attribute's `postgres` capability generate the bindings, so domain types stay typed until
the driver boundary. A structured value read in one statement, such as a Work Context's
membership rules, may arrive as `jsonb` built by the query and decode into its typed
record.

## Testing

`veoveo-test-fixtures` starts one Postgres server per test process from the PostgreSQL 18
programs on `PATH`, listening on a loopback port with a private socket directory. The
first fixture on a machine migrates a template database and keeps its data directory;
each process copies it, and each `TestDb` clones a fresh database from the template and
drops it when the test ends. The development image installs the PGDG `postgresql-18`
packages and puts their binaries on `PATH`.
