# Platform Store

`veoveo-platform-store` is the PostgreSQL access layer every Veoveo component shares.
The [database design](../../docs/DATABASE.md) is normative; this document names what
the crate implements.

## Standards And Protocols

| Standard or interface | Profile |
|---|---|
| PostgreSQL 18 | Server 18.6; one database per installation, kernel schemas `kernel` and `identity` |
| sqlx 0.9.0 | `PgPool`, `PgListener`, compile-time checked `query_file!` macros, `Migrator` with this crate's `sqlx.toml` |
| `postgres://` connection URIs | The URL names host and database and may name the user; the password travels separately |
| `LISTEN` / `NOTIFY` | `kernel.notify_change` payloads: `{"table", "operation", "key", "tenant"}` on the schema's channel |
| Advisory locks | Instances lock `(1987471726, id)`; singletons lock `(1987474279, hashtext(name))` |

## Contents

| Item | Purpose |
|---|---|
| `StoreConfig`, `PlatformStore::connect` | Pool settings and the connection pool |
| `PlatformStore::migrate`, `MIGRATOR` | Kernel migrations from `migrations/`, history in `kernel._sqlx_migrations` |
| `ensure_server`, `create_server_login`, `drop_server_login` | A registered server's group role, schema and logins, for the Gateway |
| `migrate_server` | A server's own migrations in its schema, as its group role |
| `Instance`, `reap_instances`, `instance_alive` | Process liveness through `kernel.instance` and its advisory lock |
| `Singleton` | One replica per role, on the connection that holds the lock |
| `listen`, `ChangeSignal` | Wake signals, with `Reconcile` at start and after every reconnection |
| `ensure_identity`, `work_context_by_key` | Identity records for the Gateway and registered servers |

Migration versions are grouped by owner schema: `0100` kernel, `0200` identity,
`0300` gateway, `0400` tasks and `0500` audit. Each owner adds versions within its
range.

Every static statement is a `.sql` file beside the module that runs it. The query
metadata lives in the workspace's `.sqlx` directory, and builds set `SQLX_OFFLINE=true`.
Statements whose text depends on runtime names, such as creating a login, are PL/pgSQL
functions in the migrations that quote with `format('%I')` and `format('%L')`.

## Tests

The integration tests use the [PostgreSQL fixture](../../testing/fixtures/DESIGN.md).
They cover kernel migrations, server roles and row-level security, instance liveness and
reaping, singletons, notifications across a reconnect, identity conflicts, Work Context
reads and the `postgres` macro bindings.
