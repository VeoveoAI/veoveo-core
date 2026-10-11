# veoveo-core

veoveo-core holds the contracts, protocol clients, conformance suites and shared libraries that Veoveo component repositories compile against. It runs no service.

This repository is a skeleton created from
[veoveo-template-mcp-rust](https://github.com/VeoveoAI/veoveo-template-mcp-rust). It
owns its toolchain, gate configuration, environment image and OpenShell policy. The
placeholder crate in `crates/veoveo-core` gives the gates something to check until
shared code arrives.

## Gate

```sh
docker build --target check .
```

The `Dockerfile` defines the `dev` environment and the `check` gate.

## Development environment

```sh
docker build --target dev -t veoveo-core:dev .
openshell sandbox create --name vv-core --from veoveo-core:dev \
  --policy openshell/policy.yaml --env HOME=/sandbox --detach -- sleep infinity
```

Sandbox names use the short form `vv-core`, because OpenShell limits sandbox
names to 19 characters.

## License

Apache-2.0. See [LICENSE](LICENSE).
