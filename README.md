# veoveo-core

veoveo-core holds the contracts, protocol clients, conformance suites and shared
libraries that Veoveo component repositories compile against. It runs no service.
Components consume its crates as Git dependencies, every core crate pinned to one
revision of this repository.

## Crates

| Path | Crate | Contents |
|---|---|---|
| `platform/types` | `veoveo-types` | Identifiers, vocabularies, resource addresses and shared scalar types |
| `platform/macros` | `veoveo-macros` | The `id`, `resource_address`, `Vocabulary` and `embedded_document!` macros |
| `platform/store` | `veoveo-platform-store` | PostgreSQL access: the connection pool, kernel migrations, process instances, singletons, change notifications, registered servers' roles and the Gateway's state |
| `platform/oauth` | `veoveo-oauth-client` | RS256 client-assertion signing |
| `platform/http` | `veoveo-http` | Shared HTTP admission |
| `platform/task-runtime` | `veoveo-task-runtime` | Durable MCP Tasks in PostgreSQL: claims, recovery, owner reads, subscriptions and the hosted Tasks adapter |
| `platform/task-runtime/contract` | `veoveo-task-contract` | The Task vocabulary |
| `platform/audit` | `veoveo-audit` | The audit log in PostgreSQL: group-committed appends, partition-scoped reads and change streams, the singleton sealer, signed blocks, export, retention and verification |
| `platform/audit/contract` | `veoveo-audit-contract` | Audit record, target, block and reader types, and the append inside a caller's PostgreSQL transaction |
| `platform/artifacts/contract` | `veoveo-artifact-contract` | Artifact identities, metadata and release states |
| `platform/installation/contract` | `veoveo-installation-contract` | The component lock that pins each component an installation runs to a repository and Git revision |
| `testing/fixtures/library` | `veoveo-test-fixtures` | Test fixtures for consuming repositories: a disposable PostgreSQL database, the database conformance checks and tool-input cases |

Core names no specific server, and neither does an installation's configuration.
The MCP layer, including the server registration protocol, lives in
[`veoveo-sdk-rust`](https://github.com/VeoveoAI/veoveo-sdk-rust).

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
nextest` with every feature, so feature-gated suites such as the Gateway store tests and
the database conformance checks run too. The same commands run inside the development
environment.

Tests that need a database start a private PostgreSQL cluster per test process from
the PostgreSQL 18 programs on `PATH`. The `dev` image installs the PGDG
`postgresql-18` 18.6 packages and `sqlx-cli` 0.9.0; on macOS, Homebrew's
`postgresql@18` provides the same programs.

## Dependency Policy

`deny.toml` allows Git dependencies only from the `VeoveoAI` organization. `rmcp` comes
from `VeoveoAI/rust-sdk`, a fork of the MCP Rust SDK pinned to one revision. JWT
signatures use the AWS-LC provider. sqlx 0.9.0 connects to PostgreSQL over Rustls on
the ring provider and trusts the platform's root certificates.

## Development Environment

```sh
docker build --target dev -t veoveo-core:dev .
openshell sandbox create --name vv-core --from veoveo-core:dev \
  --policy openshell/policy.yaml --detach -- sleep infinity
```

## License

Apache-2.0. See [LICENSE](LICENSE).
