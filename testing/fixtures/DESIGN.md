# Shared Native Test Fixtures

## Standards And Protocols

| Boundary | Profile |
|---|---|
| PostgreSQL 18 | The `postgres`, `initdb`, `pg_ctl`, `createdb` and `dropdb` programs of one PGDG or Homebrew 18.x installation on `PATH`; one private cluster per test process on a loopback port with a private socket directory |
| sqlx 0.9.0 | Superuser sessions over the socket for administration; `PlatformStore` pools over TCP with SCRAM passwords for tests; the offline query metadata format of a `.sqlx` directory (`query`, `hash`) |
| [pg_query](https://github.com/pganalyze/pg_query.rs) 6.2.1 | The parse tree and scanner of libpg_query, PostgreSQL 17's parser built as a library, for the database conformance checks; statements that need PostgreSQL 18 grammar fail to parse and are reported |
| `EXPLAIN (GENERIC_PLAN, VERBOSE, FORMAT JSON)`, `pg_restore_relation_stats` | PostgreSQL 18 generic plans with `enable_seqscan = off`, read for scan nodes, after singleton tables receive one-row statistics |
| Rust test harness | Source modules reused by owning tests; no separate smoke process or assertion framework |

## PostgreSQL

`store.rs` (feature `store` of `veoveo-test-fixtures`) gives each test its own database.
`TestDb::new()` clones it from a migrated template and connects two owner pools, `a`
and `b`, as two Gateway replicas would. The owner is `veoveo_owner`, a non-superuser
role with `CREATEROLE` that owns the database, so grants and row-level security apply
as in an installation.

| API | Use |
|---|---|
| `TestDb::new()` | A fresh database and two owner pools |
| `url()`, `password()`, `config()` | The owner's `postgres://` URL without password, the password, and both as a `StoreConfig`, for services a test starts |
| `connect()` | Another owner pool |
| `server_store(&slug)` | Calls `ensure_server` and `create_server_login` and returns a pool connected as the new login |
| `migrate_server(&slug, migrator)` | Applies a server's migrations in its schema as one of its logins |

The first fixture on a machine runs `initdb`, starts a server, creates the owner and
the `veoveo_template` database, applies the kernel migrations and stops the server.
The resulting data directory is kept under `$TMPDIR/vvpg/template-<key>`, where the key
hashes the PostgreSQL version and the kernel migrations' checksums. A file lock
serializes the build across processes. Each test process copies the template, starts
`postgres` with `fsync`, `synchronous_commit` and `full_page_writes` off, and sets a
random owner password through the superuser socket. `createdb --template` then clones a
database per `TestDb`, and dropping the fixture runs `dropdb --force`.

The cluster lives as long as one fixture of the process holds it. The last one stops
the server with `pg_ctl stop --mode=immediate`, which also releases its shared memory,
and removes the cluster directory. A process holds a file lock in its cluster directory
for the cluster's lifetime, and the next fixture to start stops and removes clusters
whose lock is free, such as those of a test process that was killed.

Server start tries up to five free ports and waits up to 30 seconds per attempt; other
programs and the template build each have 60 seconds, and a whole fixture setup 120.
Failures name the step and the cause, include the tail of the server log for startup
failures, and never include a password.

## Database Conformance

`database.rs` (feature `database-conformance`) holds a crate to the
[server schema rules](../../docs/DATABASE.md#server-schema-rules) from one test. Only test
code enables the feature, so `pg_query` and the C library it builds never link into a
service. Core's own suites include the file by path next to `store.rs`.

| API | Use |
|---|---|
| `Conformance::new()` | An empty check |
| `server(&slug, migrator)` | Applies the server's migrations as one of its logins, checks the server's schema and plans every query as that login; without it, queries are planned as the installation owner |
| `schema(name)` | Another schema whose tables the index-use and schema rules cover, such as a kernel schema |
| `queries_from(dir)` | Every `query-*.json` of a sqlx offline directory, each named by the `.sql` file under the directory's parent whose SHA-256 is its hash |
| `sources(dir)` | Keeps the queries whose `.sql` file lies under `dir`; repeatable |
| `query(label, sql)` | One more statement |
| `tenant_columns(table, columns)` | The columns that scope a table, in place of `tenant` and `tenant_id` |
| `all_tenants(table, reason)` | A table queries may read across tenants, reported as an exemption |
| `pending(rule, subject, reason)` | Known violations of one rule under one query file or table, reported as pending |
| `check()`, `run()` | Panics with the report when a rule is broken, or returns the `Report` |

The check clones a fixture database, reads the catalog through the owner pool and the
search path of the planning session, and analyzes each query once. The tenant and
row-bound rules work on pg_query's parse tree: a query level records its relations and
the equalities of its WHERE and join conditions, each limited to the relations it can
filter, and solves which relations are tenant-scoped and which are bounded until no
equality adds one. A value equal to a bind parameter, to the tenant column of a scoped
relation, or to the single-column primary key of a scoped row carries that fact through
subqueries, WITH queries and RETURNING lists. The planning session runs `SET
enable_seqscan = off` and then `EXPLAIN` on each statement; utility statements are
skipped. The fixture's tables are empty and never analyzed, so the planner costs each as
ten pages of rows; `pg_restore_relation_stats` gives each singleton table its one row, so
a join reads the singleton first. The whole check has 300 seconds.

The report groups violations by rule and names each query by its file, a table by
`schema.table`, and lists every exemption with its reason. A marker, an `all_tenants`
entry or a `pending` entry that exempts nothing is a violation, so exemptions disappear
with the defects they covered. PL/pgSQL and SQL function bodies are not analyzed; the query that calls a
function is.
