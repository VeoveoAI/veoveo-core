# Agent Instructions

These rules apply to veoveo-core.

- Use upstream runtimes such as OpenShell unmodified. Adapt through supported
  configuration.
- Every name, identifier, format, wire field and protocol element follows a published
  standard or [`docs/CONVENTIONS.md`](docs/CONVENTIONS.md). A name that fits neither is a
  defect. Extend a standard only through its own extension mechanism.
- Prefer strong types: typed structs, enums and domain identifiers over raw strings and
  JSON wherever the shape is known.
- Keep test logic in Rust and tool behavior in each tool's own configuration. Do not add
  a task runner or shell scripts that hold test logic.
- Pin exact versions and image digests. When changing a pin, verify the latest stable
  release at its source.
- Core names no specific server. Server-owned actions, targets, schemas and tests belong
  in the server's repository and reach the Gateway through registration.
- `mcp/contract/src/hosting` and `platform/task-runtime/src/hosting.rs` are protected:
  changes to them need the repository owner's review.
- Fixtures shared with other repositories live in `testing/fixtures` and are exported by
  `veoveo-test-fixtures`. Core's own tests include the same files, so each fixture has
  one source.
- Run `docker build --target check .` on macOS and on Linux before pushing.
- Commit small, coherent changes. Commits carry no AI or tool attribution.
