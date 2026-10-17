# Platform Store

`veoveo-store` is the PostgreSQL access layer every Veoveo component shares.
The [database design](../../docs/DATABASE.md) is normative; this document names what
the crate implements.

## Standards And Protocols

| Standard or interface | Profile |
|---|---|
| PostgreSQL 18 | Server 18.6; one database per installation, kernel schemas `kernel`, `identity`, `tasks` and `audit` |
| sqlx 0.9.0 | `PgPool`, `PgListener`, compile-time checked `query_file!` macros, `Migrator` with this crate's `sqlx.toml` |
| `postgres://` connection URIs | The URL names host and database and may name the user; the password travels separately |
| `LISTEN` / `NOTIFY` | `kernel.notify_change` payloads: `{"table", "operation", "key", "tenant"}` on the schema's channel, or only on `<schema>.<qualifier>` when the trigger names a qualifier column |
| Advisory locks | Instances lock `(1987471726, id)`; singletons lock `(1987474279, hashtext(name))`; first-time identity creation takes the transaction lock `(1987476603, hashtext(id))` |
| [RFC 4648](https://www.rfc-editor.org/rfc/rfc4648) §5 base64url, JSON | Keyset cursors: unpadded base64url over the JSON object `{"scope", "position"}`; `scope` is a `sha256:` digest under the domain `https://veoveo.ai/formats/keyset-cursor-scope` |

## Contents

| Item | Purpose |
|---|---|
| `StoreConfig`, `PlatformStore::connect` | Pool settings and the connection pool |
| `PlatformStore::migrate`, `MIGRATOR` | Kernel migrations from `migrations/`, history in `kernel._sqlx_migrations` |
| `ensure_server`, `create_server_login`, `drop_server_login` | A registered server's group role, schema and logins, for the Hub |
| `migrate_server` | A server's own migrations in its schema, as its group role |
| `Instance`, `reap_instances`, `instance_alive` | Process liveness through `kernel.instance` and its advisory lock, reconnected after a lost connection, and reaped once the lock stayed free for the instance grace |
| `InstanceGrace`, `INSTANCE_GRACE`, `StoreConfig::instance_grace` | How long a lock stays free before a reap, in whole milliseconds, 30 seconds by default |
| `Singleton` | One replica per role, on the connection that holds the lock |
| `listen`, `Channel`, `ChangeSignal` | Wake signals on schema or qualified channels, with `Reconcile` at start and after every reconnection |
| `ensure_identity`, `work_context_by_key` | Identity records for the Hub and registered servers |
| `MaintenanceBatch`, `Pruned` | Batched deletion of expired rows, described below |
| `PageSize<MAX>`, `Page` | A page size from 1 to the contract's `MAX`, read with `LIMIT size + 1` so the extra row decides whether a next cursor exists |
| `CursorScope`, `KeysetCursor`, `KeysetCodec` | An opaque keyset cursor over `OpaqueCursor`, bound to the listing, tenant and filter digest that issued it |

Migration versions are grouped by owner schema in hundreds: `01xx` kernel, `02xx`
identity, `04xx` tasks and `05xx` audit. Each owner adds versions within its range. A
component that owns another schema, such as the Hub, keeps that schema's migrations
and their history in its own repository.

Every static statement is a `.sql` file beside the module that runs it. The query
metadata lives in the workspace's `.sqlx` directory, and builds set `SQLX_OFFLINE=true`.
Statements whose text depends on runtime names, such as creating a login, are PL/pgSQL
functions in the migrations that quote with `format('%I')` and `format('%L')`.

## Batched Maintenance

A statement that deletes or clears expired rows changes at most one `MaintenanceBatch`
of them, 1 to 10,000 rows and 1,000 by default. It selects the batch in expiry order
through the expiry index and locks it with `FOR UPDATE SKIP LOCKED`, so each batch is
one short transaction and replicas that run the same pass take different rows.
`Pruned::in_batches` repeats the statement until a batch changes fewer rows than it
holds and reports the rows and batches the pass took. Every prune in this crate and the
Task runtime's retention run this way.

## Tests

The integration tests use the [PostgreSQL fixture](../../testing/fixtures/DESIGN.md).
They cover kernel migrations, server roles and row-level security, instance liveness and
reaping, a reap that marks a free lock and deletes the instance only after the grace, a
mark that taking the lock back clears, an instance that takes its lock back after its session ends, continues under a
new id after its row was reaped, ends the earlier session of its own login, refuses
another login's resumption and ends once its login is dropped, singletons,
notifications across a reconnect, identity conflicts, Work Context
reads and the `postgres` macro bindings.
`tests/database_conformance.rs` holds every query under `src` and the `kernel` and
`identity` schemas to the
[server schema rules](../../docs/DATABASE.md#server-schema-rules).
