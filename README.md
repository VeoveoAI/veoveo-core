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
| `platform/policy` | `veoveo-policy` | The shared policy evaluator and role evaluation over registered servers' permissions |
| `platform/oauth` | `veoveo-oauth-client` | RS256 client-assertion signing |
| `platform/http` | `veoveo-http` | Shared HTTP admission |
| `platform/task-runtime/contract` | `veoveo-task-contract` | The Task vocabulary |
| `platform/audit/contract` | `veoveo-audit-contract` | Audit record, target and detail types |
| `platform/artifacts/contract` | `veoveo-artifact-contract` | Artifact identities, metadata and release states |
| `platform/artifacts/client` | `veoveo-artifact-client` | HTTP client for the Artifact plane: objects, streamed transfers, write capabilities, grants, share links, release states and access requests |
| `platform/knowledge/contract` | `veoveo-knowledge-contract` | Knowledge source and retrieval types |
| `platform/runtimes/embedding/contract` | `veoveo-embedding-contract` | Embedding request and profile types |
| `platform/gateway/contract` | `veoveo-gateway-contract` | Gateway catalog registry, actions and admission types |
| `mcp/contract` | `veoveo-mcp-contract` | The hosted MCP server contract and its hosting runtime |
| `mcp/ext-apps` | `veoveo-mcp-ext-apps` | The MCP Apps extension |
| `mcp/ext-knowledge` | `veoveo-mcp-ext-knowledge` | The `ai.veoveo/knowledge-source` extension |
| `mcp/ext-registration` | `veoveo-mcp-ext-registration` | The server registration protocol: the `ai.veoveo/registration` and `ai.veoveo/authorization` declarations, RFC 7591 and RFC 7592 registration types, the database grant, granted-permission details and the registration client servers run |
| `mcp/conformance` | `veoveo-mcp-conformance` | Hosted server conformance checks |
| `testing/fixtures/modular-mcp` | `veoveo-modular-fixture-mcp` | An independently typed server used as a conformance and Gateway test peer |
| `testing/fixtures/library` | `veoveo-test-fixtures` | Test fixtures for consuming repositories: tool-input cases |

Core names no specific server, and neither does an installation's configuration.
Servers register with the Gateway through the
[server registration protocol](mcp/ext-registration/DESIGN.md), and the
installation grants roles that select their declared permissions by access kind and
risk.

## Conventions

Every name and format in Veoveo repositories follows a published standard or
[`docs/CONVENTIONS.md`](docs/CONVENTIONS.md).

## Gate

```sh
docker build --target check .
```

The check stage runs `cargo fmt`, `cargo clippy`, `cargo deny`, `cargo hack` and
`cargo nextest`. The same commands run inside the development environment.

## Dependency Policy

`deny.toml` allows Git dependencies only from the `VeoveoAI` organization. `rmcp` comes
from `VeoveoAI/rust-sdk`, a fork of the MCP Rust SDK pinned to one revision. JWT
signatures use the AWS-LC provider.

## Development Environment

```sh
docker build --target dev -t veoveo-core:dev .
openshell sandbox create --name vv-core --from veoveo-core:dev \
  --policy openshell/policy.yaml --detach -- sleep infinity
```

## License

Apache-2.0. See [LICENSE](LICENSE).
