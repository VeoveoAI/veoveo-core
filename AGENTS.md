# Agent Instructions

These rules apply to veoveo-core.

- Use upstream runtimes such as OpenShell and PostgreSQL unmodified. Adapt through
  supported configuration.
- Every name, identifier, format, wire field and protocol element follows a published
  standard or [`docs/CONVENTIONS.md`](docs/CONVENTIONS.md). A name that fits neither is a
  defect. Extend a standard only through its own extension mechanism.
- Prefer strong types: typed structs, enums and domain identifiers over raw strings and
  JSON wherever the shape is known.
- Keep test logic in Rust and tool behavior in each tool's own configuration. Do not add
  a task runner or shell scripts that hold test logic.
- Pin exact versions and image digests. When changing a pin, verify the latest stable
  release at its source.
- Core is protocol-independent and names no specific server or domain. MCP code belongs
  in `veoveo-sdk-rust`; server-owned actions, audit classes and details, targets,
  schemas and tests belong in the server's repository and reach the Gateway through
  registration. Do not add a dependency on rmcp or on a `veoveo-mcp-*`,
  `veoveo-sdk-*` or `veoveo-service-*` crate.
- Fixtures shared with other repositories live in `testing/fixtures` and are exported by
  `veoveo-test-fixtures`. Core's own tests include the same files, so each fixture has
  one source.
- The database follows [`docs/DATABASE.md`](docs/DATABASE.md). Every static query is a
  `.sql` file run through sqlx's `query_file!` macros. After changing a query or a
  migration, regenerate `.sqlx` with `cargo sqlx prepare --workspace -- --all-targets
  --all-features` against a migrated database.
- Tests that need PostgreSQL use the store fixture, which runs a private `postgres` from
  the PostgreSQL 18 programs on `PATH`.
- Run `docker build --target check .` on macOS and on Linux before pushing.
- Commit small, coherent changes. Commits carry no AI or tool attribution.
