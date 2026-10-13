# veoveo-core

veoveo-core holds the protocol-independent platform libraries that Veoveo components
compile against: shared types and macros, the PostgreSQL kernel, identity, durable
Tasks and the audit log. It runs no service, depends on no MCP library and names no
server. Components consume its crates as Git dependencies, every core crate pinned to
one revision of this repository.

## Crates

| Path | Crate | Contents |
|---|---|---|
| `platform/types` | `veoveo-types` | Identifiers, vocabularies, resource addresses and shared scalar types |
| `platform/macros` | `veoveo-macros` | The `id`, `resource_address`, `Vocabulary` and `embedded_document!` macros |
| `platform/store` | `veoveo-platform-store` | PostgreSQL access: the connection pool, kernel and identity migrations, process instances, singletons, change notifications, registered servers' roles and schemas, keyset pages and batched maintenance |
| `platform/oauth` | `veoveo-oauth-client` | RS256 client-assertion signing |
| `platform/http` | `veoveo-http` | Shared HTTP admission |
| `platform/task-runtime` | `veoveo-task-runtime` | Durable Tasks in PostgreSQL: claims, recovery, durable steps, owner reads, inputs, subscriptions and retention |
| `platform/task-runtime/contract` | `veoveo-task-contract` | The Task vocabulary |
| `platform/audit` | `veoveo-audit` | The audit log in PostgreSQL: group-committed appends, partition-scoped reads and change streams, the singleton sealer, signed blocks, export, retention and verification |
| `platform/audit/contract` | `veoveo-audit-contract` | The audit record envelope, owner-declared classes and details, targets, blocks and reader types, and the append inside a caller's PostgreSQL transaction |
| `platform/installation/contract` | `veoveo-installation-contract` | The component lock that pins each component an installation runs to a repository and Git revision |
| `testing/fixtures/library` | `veoveo-test-fixtures` | Test fixtures for consuming repositories: a disposable PostgreSQL database and the database conformance checks |

Core names no protocol method, server or domain. The Rust MCP layer, including hosting,
the client servers use to call each other and the server registration protocol, lives
in [`veoveo-sdk-rust`](https://github.com/VeoveoAI/veoveo-sdk-rust). Each server
publishes its own contract through its crate's `contract` feature, and each owner
declares its audit classes and details. The core boundary test fails the gate when a
core crate depends on rmcp or on a `veoveo-mcp-*`, `veoveo-sdk-*` or `veoveo-service-*`
crate.

## Conventions

Every name and format in Veoveo repositories follows a published standard or
[`docs/CONVENTIONS.md`](docs/CONVENTIONS.md).

## Gate

```sh
docker build --target check .
```

The check stage first applies the kernel migrations to a scratch PostgreSQL database
and runs `cargo sqlx prepare --workspace --check`, so query metadata in `.sqlx` that no
longer matches the queries fails the gate. It then runs `cargo fmt`, `cargo clippy`
with every feature, `cargo deny`, `cargo hack` with each feature on its own, and `cargo
nextest` with every feature, so feature-gated suites such as the database conformance
checks run too. The same commands run inside the development
environment.

Tests that need a database start a private PostgreSQL cluster per test process from
the PostgreSQL 18 programs on `PATH`. The `dev` image installs the PGDG
`postgresql-18` 18.6 packages and `sqlx-cli` 0.9.0; on macOS, Homebrew's
`postgresql@18` provides the same programs.

## Dependency Policy

`deny.toml` allows Git dependencies only from the `VeoveoAI` organization; core itself
has none. JWT signatures use the AWS-LC provider. sqlx 0.9.0 connects to PostgreSQL over Rustls on
the ring provider and trusts the platform's root certificates.

## Development Environment

```sh
docker build --target dev -t veoveo-core:dev .
openshell sandbox create --name vv-core --from veoveo-core:dev \
  --policy openshell/policy.yaml --detach -- sleep infinity
```

## License

Apache-2.0. See [LICENSE](LICENSE).
