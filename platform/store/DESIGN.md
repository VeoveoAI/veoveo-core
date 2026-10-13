# Platform Store

`veoveo-platform-store` is the PostgreSQL access layer every Veoveo component shares.
The [database design](../../docs/DATABASE.md) is normative; this document names what
the crate implements.

## Standards And Protocols

| Standard or interface | Profile |
|---|---|
| PostgreSQL 18 | Server 18.6; one database per installation, kernel schemas `kernel`, `identity`, `gateway`, `tasks` and `audit` |
| sqlx 0.9.0 | `PgPool`, `PgListener`, compile-time checked `query_file!` macros, `Migrator` with this crate's `sqlx.toml` |
| `postgres://` connection URIs | The URL names host and database and may name the user; the password travels separately |
| `LISTEN` / `NOTIFY` | `kernel.notify_change` payloads: `{"table", "operation", "key", "tenant"}` on the schema's channel, or only on `<schema>.<qualifier>` when the trigger names a qualifier column |
| Advisory locks | Instances lock `(1987471726, id)`; singletons lock `(1987474279, hashtext(name))` |
| [RFC 4648](https://www.rfc-editor.org/rfc/rfc4648) §5 base64url, JSON | Keyset cursors: unpadded base64url over the JSON object `{"scope", "position"}`; `scope` is a `sha256:` digest under the domain `https://veoveo.ai/formats/keyset-cursor-scope` |

## Contents

| Item | Purpose |
|---|---|
| `StoreConfig`, `PlatformStore::connect` | Pool settings and the connection pool |
| `PlatformStore::migrate`, `MIGRATOR` | Kernel migrations from `migrations/`, history in `kernel._sqlx_migrations` |
| `ensure_server`, `create_server_login`, `drop_server_login` | A registered server's group role, schema and logins, for the Gateway |
| `migrate_server` | A server's own migrations in its schema, as its group role |
| `Instance`, `reap_instances`, `instance_alive` | Process liveness through `kernel.instance` and its advisory lock |
| `Singleton` | One replica per role, on the connection that holds the lock |
| `listen`, `Channel`, `ChangeSignal` | Wake signals on schema or qualified channels, with `Reconcile` at start and after every reconnection |
| `ensure_identity`, `work_context_by_key` | Identity records for the Gateway and registered servers |
| `MaintenanceBatch`, `Pruned` | Batched deletion of expired rows, described below |
| `PageSize<MAX>`, `Page` | A page size from 1 to the contract's `MAX`, read with `LIMIT size + 1` so the extra row decides whether a next cursor exists |
| `CursorScope`, `KeysetCursor`, `KeysetCodec` | An opaque keyset cursor over `OpaqueCursor`, bound to the listing, tenant and filter digest that issued it |
| `gateway` module, `gateway` feature | The Gateway's state, typed with the Gateway contracts; described below |

Migration versions are grouped by owner schema in hundreds: `01xx` kernel, `02xx`
identity, `03xx` gateway, `04xx` tasks and `05xx` audit. Each owner adds versions within
its range.

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

## Gateway State

The `gateway` schema holds what every Gateway replica shares. Only the Gateway, as the
installation owner, reads or writes it; registered servers receive no grant on it.

| Tables | Holds |
|---|---|
| `control_revision`, `control_object` | Published installation configurations and their objects, immutable once written |
| `control_active` | One row naming the active revision, or none before the first publication |
| `jwt_revocation`, `replay_id` | Revoked JWTs and single-use `jti` values, each kept until the JWT expires |
| `authorization_request`, `authorization_code` | Hosted authorization requests waiting for the identity provider, and issued codes |
| `refresh_family`, `refresh_token` | Rotating refresh-token families and the SHA-256 digest of each generation |
| `namespace_claim`, `registration` | The key each server's namespace is pinned to, and each registration with its login |
| `task_route` | The public route of each upstream Task the Gateway projected |

With the `gateway` feature the crate exposes the statements the Gateway's own state
needs:

| API | Behavior |
|---|---|
| `claim_replay_id`, `prune_replay_ids` | `INSERT … ON CONFLICT DO UPDATE … WHERE` the earlier claim expired, so one caller claims each identifier |
| `record_jwt_revocation`, `jwt_revocation`, `prune_jwt_revocations` | Upsert by profile, issuer and `jti`; reads ignore expired revocations |
| `record_authorization_request`, `consume_authorization_request` | Consumption deletes the row, so one caller receives it |
| `record_authorization_code`, `consume_authorization_code` | Redemption sets `consumed_at` once |
| `create_refresh_family`, `refresh_grant`, `refresh_family` | A family with its generation-0 token, and reads by token digest or family |
| `rotate_refresh_token` | Locks the family row, then rotates, redelivers the successor inside its delivery window, or revokes the family as a replay |
| `revoke_refresh_family` | RFC 7009 revocation bound to the family's client; repeating it succeeds |
| `prune_refresh_state`, `clear_refresh_delivery_envelopes` | Retention of delivery envelopes, tokens and families. Deleting a token clears its predecessor's `replacement` through the `refresh_token_replacement` index |
| `prune_replay_ids`, `prune_jwt_revocations`, `prune_authorization_records`, `prune_task_routes` | Deletion of expired identifiers, revocations, requests and codes, and Task routes; the Gateway schedules each pass |
| `active_control_revision`, `publish_control_revision` | Publication locks the active pointer, fails with `ControlRevisionChanged` when another revision became active, and replaces the identity schema's Work Contexts in the same transaction |
| `artifact_upload_authority` | The active configuration digest, the Work Context's policy revision and digest, and the profile's policy digest |

Every change to a family's tokens holds the family's row lock, so concurrent
presentations of one refresh token serialize across replicas. Rotation and publication
take a `TransactionAppend`, rows another owner records in the same transaction; the
Gateway passes its audit records, and a refused append rolls the change back.

`registration` notifies the `gateway` channel when a row is inserted or deleted or its
metadata changes. Its `orphaned_at` is null while the registration's login holds a live
kernel instance. The Gateway sets it when it first finds the login without one, clears
it when an instance appears, and removes the registration with its login once
`orphaned_at` is older than the grace period. An installation whose servers stay alive
therefore writes nothing while it is idle, and neither mark wakes a listener.

A `task_route` references its tenant, owner and Work Context in `identity` and goes with
the Work Context when a publication removes it, which `task_route_work_context` makes a
lookup rather than a scan. `task_route_owner` and `task_route_tenant` do the same for
the foreign-key checks when a principal or a tenant is deleted. `prune_task_routes`
deletes the routes whose `expires_at` has passed, through `task_route_expiry`.

## Tests

The integration tests use the [PostgreSQL fixture](../../testing/fixtures/DESIGN.md).
They cover kernel migrations, server roles and row-level security, instance liveness and
reaping, singletons, notifications across a reconnect, identity conflicts, Work Context
reads and the `postgres` macro bindings. `tests/gateway.rs` covers single-use replay
identifiers, authorization requests and codes across two pools, revocation expiry,
concurrent refresh rotation with redelivery and replay revocation, rollback on a refused
append, bound client revocation, publication conflicts, Work Context replacement and
upload authority digests, Task route removal with their Work Context and by expiry, the
registration notifications, batched pruning that two replicas share, and plans that read
the replacement, Work Context and expiry indexes with sequential scans disabled.
`tests/database_conformance.rs` holds every query under `src` and the `kernel`,
`identity` and `gateway` schemas to the
[server schema rules](../../docs/DATABASE.md#server-schema-rules).
