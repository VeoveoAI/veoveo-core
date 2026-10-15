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
| Advisory locks | Process instance liveness, singleton election and first-time identity creation |
| `SELECT … FOR UPDATE SKIP LOCKED` | Work claims |
| Row-level security, roles and `GRANT` | Isolation between the kernel and registered servers |
| `xid8` and `pg_snapshot_xmin` | Commit-safe ordering for the audit log |
| `COMMENT ON TABLE` | Every table's comment states its retention, starting `Retention:` |
| [pg_query](https://github.com/pganalyze/pg_query.rs) 6.2.1 and `EXPLAIN (GENERIC_PLAN)` | Test-only server schema rules: the parse tree of libpg_query, PostgreSQL 17's parser, and PostgreSQL 18 generic plans |

## Database, Schemas And Roles

An installation has one database. Each owner has one schema:

| Schema | Owner | Contents |
|---|---|---|
| `kernel` | Core | Migration history, process instances and shared functions |
| `identity` | Core | Enterprises, tenants, principals and Work Contexts |
| `gateway` | The Gateway | Control-plane revisions, OAuth state, registrations and Task routes, from the Gateway's own migrations |
| `tasks` | Core | Tasks, their input exchanges, steps, operations with their provider events, and idempotency keys |
| `audit` | Core | Declared audit classes, audit records, blocks, seals and export cursors; servers declare and append through two functions |
| `usage` | Core | Declared usage kinds, monthly-partitioned usage records with their idempotency keys, and daily totals; servers declare and append through two functions |
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
  random password. Deregistration drops it, and so does the Gateway when the login has
  held no live process instance for the installation's grace period.
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
`SECURITY DEFINER` functions granted to `veoveo_server`. Such a function runs as its
owner, so it asks `kernel.servers_of(session_user)` for the servers of the calling login
instead. Every function in a kernel schema is revoked from `PUBLIC`, and the migrations
grant each one that servers call.

`identity.ensure_identity` records an identity on every login, so concurrent first logins
of one principal call it at the same time. Its inserts arbitrate on the deterministic
identifier, and each table also has a unique natural key, such as a principal's tenant and
principal key, that the insert does not arbitrate. Before creating a missing row the
function takes the transaction advisory lock `(1987476603, hashtext(id))`, so the first
caller creates it and the others find the committed row. A call for rows that exist
takes no lock and writes nothing unless the display name changes.

Servers hold no privilege on the audit tables. A server login declares its audit classes
with `audit.declare_server_classes(classes, schemas)`, which admits only classes under
the `<server>.` prefix of one of its servers and stores that server with each class, and
appends with `audit.append_server_records(drafts, occurred)`, which admits only records
of classes its servers declared. Both are `SECURITY DEFINER` functions owned by the
installation owner, with `search_path` pinned and `EXECUTE` granted to `veoveo_server`
only. The append runs the owner's `audit.append` inside the server's transaction, so a
server's records commit with its change and take the ordering below.

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
`kernel`, `02xx` for `identity`, `04xx` for `tasks`, `05xx` for `audit` and `06xx` for
`usage`. The Gateway's
migrations live in its repository and record their history in its own schema. A
server's migrations live in its own crate and record their history in
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
the connection. A clean shutdown deletes the row and releases the lock before it closes
the connection, so the instance reads as dead once shutdown returns.

Work that a process owns references its instance through `ON DELETE SET NULL`. Each
live process calls `kernel.reap_instances(grace)` when it starts and every 15 seconds.
The first call that finds an instance's lock free records the time in
`lock_free_since`; a later call deletes the row once the lock has stayed free for the
grace, 30 seconds by default. A call that finds the lock held again clears the mark, and
so does `kernel.resume_instance`, so a process whose connection dropped for a moment
keeps its instance. The cascade clears ownership in the same transaction as the delete,
and a trigger moves the released work to its recovery state. While any process runs,
a crashed process's work reaches recovery within the grace and two reap intervals.
Settlement writes compare the owner instance, so a process whose instance was reaped
cannot settle work it no longer owns.

The grace belongs to each process's store configuration, so the shortest grace any
process of an installation uses decides; installations give every process the same one.
It stays shorter than the Gateway's 60-second registration grace.

The `Instance` type checks its connection and its row every 5 seconds. When a check
fails, takes longer than 5 seconds or finds the row gone, the instance reconnects with a
new dedicated connection, waiting 100 milliseconds after the first failed attempt and
doubling to at most 5 seconds. On the new connection it calls
`kernel.resume_instance(id)`:

| Result | Meaning | Next |
|---|---|---|
| `resumed` | This session holds the lock again and the row is there | The instance keeps its id and the work it owns |
| `held` | Another session of the same login still holds the lock | The function terminated that session, which only the process's abandoned connection can be, and the instance tries again at once |
| `gone` | A reaper deleted the row, or another login started it | The instance starts again under a new id as its next generation; the reaper already moved its earlier work to recovery |

The instance ends only on shutdown or when the database refuses its login (SQLSTATE
`28000` or `28P01`), as it does once the Gateway removed the server's registration and
dropped the login. The process then stops acting on the work it owned. A server that
loses its database connection for less than the grace therefore keeps its registration
and its work, and one that stays away longer keeps running under a new instance.

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
as the owner in the same transaction. A trigger notifies the channel of the work's server
when work is created or released, and workers also claim at startup and after each reap.

## Change Notifications

Row triggers execute `kernel.notify_change(key_column [, tenant_column [,
qualifier_column]])`, which calls `pg_notify` with a JSON payload naming the table, the
operation (`insert`, `update` or `delete`) and the row's key and tenant. A key longer than
1,024 bytes is sent as null, and an empty tenant column name sends no tenant. Each schema
has one channel named after the schema. A trigger that names a qualifier column notifies
only `<schema>.<qualifier>`, with the column's value as the qualifier. A table whose rows
each belong to one server qualifies by its `server` column, so `tasks.task` notifies
`tasks.time` for a Task of server `time`, and each server's listener wakes only for its
own rows. The qualifier column is `NOT NULL` and keeps its value for the row's life, and
a qualified channel fits PostgreSQL's 63-byte identifier limit.

A consumer listens through `PlatformStore::listen`, a `PgListener` on its own connection
that yields `Reconcile` once subscribed and again after every reconnection, because
Postgres does not replay notifications sent while it was disconnected. `Channel` admits a
schema channel or a qualified one, and `Channel::qualified` builds the latter.
Notifications are wake signals: consumers read current rows under their own authorization
and never trust payload content as data.

## Batched Maintenance

A statement that deletes or clears expired rows changes at most one batch, selected in
expiry order through an index on the expiry column and locked with `FOR UPDATE SKIP
LOCKED`. Its caller repeats it, each batch in its own short transaction, until a batch
changes fewer rows than it holds. Replicas that prune at the same time take different
rows, and no pass holds locks on a large set. A foreign key whose referencing column a
delete or cascade searches has an index on that column.

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

sqlx prepares each statement once per connection, and PostgreSQL may switch a prepared
statement to a generic plan that ignores its parameter values. A keyset bound that the
first page omits is therefore coalesced to a bound before every row, such as `(created_at,
id) > (coalesce($9, '-infinity'), coalesce($10, nil UUID))`, rather than written as
`$9 IS NULL OR …`, so the bound stays an index range in a generic plan. A statement whose
optional equality filters must each seek their own index, such as an audit page, runs in
a transaction with `SET LOCAL plan_cache_mode = force_custom_plan`. Plan tests prepare a
verbatim copy of the statement in a test script and check the plan's index conditions.

## Server Schema Rules

Every crate that owns queries, core's and each server's, holds them to four rules in one
test. `veoveo_fixtures::database::Conformance` (feature `database-conformance`)
migrates a fixture database, reads every compile-checked query from the workspace's
`.sqlx` directory, parses it with PostgreSQL's own parser through `pg_query`, plans it,
and panics with one report that lists every violation and every exemption. The checks
run only in tests, so they cost the running service nothing.

```rust
#[tokio::test]
async fn queries_and_schema_follow_the_database_rules() {
    Conformance::new()
        .server(&ServerSlug::parse("time").unwrap(), persistence::migrator())
        .queries_from(concat!(env!("CARGO_MANIFEST_DIR"), "/.sqlx"))
        .sources(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
        .check()
        .await;
}
```

`server` applies the server's migrations as one of its logins, as startup does, checks
the server's schema and plans each query as that login. `schema` adds another checked
schema, and `sources` keeps the queries whose `.sql` files lie in a directory.

| Rule | A query or table passes when |
|---|---|
| Tenant predicates | Every reference to a table with a tenant column, in FROM, JOIN, subqueries, WITH queries, UPDATE and DELETE, has an equality on that column to a bind parameter, or to the tenant column of another relation in the statement that is itself constrained. An outer join's condition constrains only its nullable side. Every INSERT supplies the tenant column unless the column is generated |
| Row bounds | A SELECT has a LIMIT, pins a primary or unique key by equality, or aggregates without GROUP BY. An UPDATE or DELETE has a WHERE clause and either pins a primary or unique key of its target or selects its rows through a subquery with a LIMIT |
| Index use | The generic plan, with `enable_seqscan = off`, reaches every row of a checked table through an index. A Seq Scan fails, and so does an index scan without an index condition, except a top-N read under a LIMIT that filters nothing and the input of a hash or merge join whose key leads an index of that table. A scan under a LIMIT fails when its filter tests a key column of its index that no index condition seeks, as an optional keyset bound does under a generic plan; a nested loop's join filter counts as the filter of its outer scan, so a bound read from a joined row instead of a scalar subquery fails too |
| Schema rules | Every table of a checked schema has a primary key, every foreign key has an index whose leading columns are the key's columns, and every table's comment states its retention |

Tenant columns are `tenant` and `tenant_id` unless the check names others with
`tenant_columns`, such as `partition` for `audit.record`. A tenant value also travels
through a statement: a column equated to a bind parameter, a WITH query's output column
that holds a constrained tenant, and the primary key of a tenant-scoped row all count. An
UPDATE whose target is pinned by the key that a tenant-scoped WITH query returned is
therefore scoped too.

A key column is pinned by an equality to a bind parameter, a constant, an uncorrelated
scalar subquery, `= ANY` of a parameter array, `IN` a bounded subquery, or a column of
another bounded relation. A partial unique index counts when its predicate only requires
columns the query pins. A table whose primary key is one boolean column checked to be
true holds at most one row, so it counts as keyed, and the index-use and foreign-key
rules skip it.

A query exempts itself from one rule with a comment that gives the reason:

| Marker | Exempts |
|---|---|
| `-- veoveo: all tenants (<reason>)` | Tenant predicates |
| `-- veoveo: unbounded (<reason>)` | Row bounds |
| `-- veoveo: scan (<reason>)` | Index use |

`all_tenants(table, reason)` exempts every query on a table that callers address by an
identifier alone, such as the Gateway's refresh-token families, which a token digest
selects. `pending(rule, subject, reason)` accepts the violations of one rule under one
query file or table as known work, such as a defect that a later change fixes. A marker
without a reason, an unknown marker and an exemption or pending entry that no finding
needs are violations, so each leaves with the defect it covered. The report lists every
exemption and pending entry with its reason, so review sees each one.

A table states its retention in its comment, `COMMENT ON TABLE time.event IS 'Retention:
…'`: how long a row stays and what removes it.

The store supplies the shapes that pass these rules. `PageSize<MAX>` admits a page size
up to the maximum the contract chooses, and its `page` turns rows read with
`LIMIT fetch_limit()`, one more than the page, into a `Page` whose next position exists
only when the extra row came back. `CursorScope` binds a `KeysetCursor`, an
`OpaqueCursor` over `KeysetCodec`, to the listing, the tenant and a digest of the
filters that produced it, so a cursor from another scope fails to parse. Deletes follow
[Batched Maintenance](#batched-maintenance) through `MaintenanceBatch` and
`Pruned::in_batches`.

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

`veoveo-fixtures` starts one Postgres server per test process from the PostgreSQL 18
programs on `PATH`, listening on a loopback port with a private socket directory. The
first fixture on a machine migrates a template database and keeps its data directory;
each process copies it, and each `TestDb` clones a fresh database from the template and
drops it when the test ends. The development image installs the PGDG `postgresql-18`
packages and puts their binaries on `PATH`.
