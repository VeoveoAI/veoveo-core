# Shared Native Test Fixtures

## Standards And Protocols

| Boundary | Profile |
|---|---|
| PostgreSQL 18 | The `postgres`, `initdb`, `pg_ctl`, `createdb` and `dropdb` programs of one PGDG or Homebrew 18.x installation on `PATH`; one private cluster per test process on a loopback port with a private socket directory |
| sqlx 0.9.0 | Superuser sessions over the socket for administration; `PlatformStore` pools over TCP with SCRAM passwords for tests |
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

## Catalog Registration

`catalog_registry.rs` reuses the production registration source in
[`platform/gateway/catalog`](https://github.com/BiomaAI/veoveo/blob/0c33cf8a7e5a1d46f953702553ac6643f99c1363/platform/gateway/catalog/DESIGN.md). Tests therefore
exercise the same kernel and owner declarations as installation binaries without
maintaining another registration list. Source reuse lets an owner test use its own
library types without a reverse package dependency on the installation recipe.

`registry()` clones one binding within the test executable. `fresh_registry()` creates
an independent binding for rejection tests: admitted handles and targets from one
registry cannot authorize requests through another. `catalog_admission.rs` supplies
the gateway admission wrapper over these declarations. Domain assertions stay in
the owning suites; the fixtures provide no policy decisions or installation credentials.

## Input Admission

`tool_inputs.rs` shares pure owner-owned input fixtures between contract consumers
and authenticated hosted tests. It checks fixture integrity, decodes serialized
bytes, and mutates controlled objects, required fields and variant tags. Owners
select their actual request types and assert domain state; the helper owns no
transport, lifecycle, provider or domain catalog.
