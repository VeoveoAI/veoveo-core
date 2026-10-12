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
| `platform/modules` | `veoveo-modules` | Module schema declarations and the migration runner |
| `platform/store` | `veoveo-platform-store` | SurrealDB access, kernel schemas, change feeds and audit storage |
| `platform/policy` | `veoveo-policy` | The shared policy evaluator |
| `platform/oauth` | `veoveo-oauth-client` | RS256 client-assertion signing |
| `platform/http` | `veoveo-http` | Shared HTTP admission |
| `platform/task-runtime` | `veoveo-task-runtime` | Durable Tasks: storage, leases, recovery and subscriptions |
| `platform/task-runtime/contract` | `veoveo-task-contract` | The Task vocabulary |
| `platform/audit` | `veoveo-audit` | Audit records, signing, indexing and export |
| `platform/audit/contract` | `veoveo-audit-contract` | Audit record, target and detail types |
| `platform/artifacts/contract` | `veoveo-artifact-contract` | Artifact identities, metadata and release states |
| `platform/artifacts/client` | `veoveo-artifact-client` | HTTP client for the Artifact plane: objects, streamed transfers, write capabilities, grants, share links, release states and access requests |
| `platform/knowledge/contract` | `veoveo-knowledge-contract` | Knowledge source and retrieval types |
| `platform/runtimes/embedding/contract` | `veoveo-embedding-contract` | Embedding request and profile types |
| `platform/gateway/contract` | `veoveo-gateway-contract` | Gateway catalog registry, actions and admission types |
| `mcp/contract` | `veoveo-mcp-contract` | The hosted MCP server contract and its hosting runtime |
| `mcp/apps-extension` | `veoveo-mcp-apps-extension` | The MCP Apps extension |
| `mcp/knowledge-extension` | `veoveo-mcp-knowledge-extension` | The `ai.veoveo/knowledge-source` extension |
| `mcp/conformance` | `veoveo-mcp-conformance` | Hosted server conformance checks |
| `testing/fixtures/modular-mcp` | `veoveo-modular-fixture-mcp` | An independently typed server used as a conformance and Gateway test peer |
| `testing/fixtures/library` | `veoveo-test-fixtures` | Test fixtures for consuming repositories: a disposable SurrealDB store and tool-input cases |

Core names no specific server. Servers contribute their own declarations through
registration with the Gateway.

## Gate

```sh
docker build --target check .
```

The check stage runs `cargo fmt`, `cargo clippy`, `cargo deny`, `cargo hack` and
`cargo nextest`. The same commands run inside the development environment.

Tests that need a database start one `surreal` server process per fixture. The `dev`
image copies the SurrealDB 3.3.0 binary from the server image pinned by digest, because
no container runtime enters a sandbox.

## Dependency Policy

`deny.toml` allows Git dependencies only from the `VeoveoAI` organization. `rmcp` comes
from `VeoveoAI/rust-sdk`, a fork of the MCP Rust SDK pinned to one revision. The
SurrealDB crates are licensed under the Business Source License 1.1, which `deny.toml`
accepts for those crates only. JWT signatures use the AWS-LC provider.

## Development Environment

```sh
docker build --target dev -t veoveo-core:dev .
openshell sandbox create --name vv-core --from veoveo-core:dev \
  --policy openshell/policy.yaml --detach -- sleep infinity
```

## License

Apache-2.0. See [LICENSE](LICENSE).
