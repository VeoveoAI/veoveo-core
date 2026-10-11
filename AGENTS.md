# Agent Instructions

These rules apply to veoveo-core.

- Use upstream runtimes such as OpenShell and SurrealDB unmodified. Adapt through
  supported configuration.
- Prefer strong types: typed structs, enums and domain identifiers over raw strings and
  JSON wherever the shape is known.
- Keep test logic in Rust and tool behavior in each tool's own configuration. Do not add
  a task runner or shell scripts that hold test logic.
- Pin exact versions and image digests. When changing a pin, verify the latest stable
  release at its source.
- Run `docker build --target check .` on macOS and on Linux before pushing.
- Commit small, coherent changes. Commits carry no AI or tool attribution.
