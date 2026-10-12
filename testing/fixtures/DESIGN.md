# Shared Native Test Fixtures

## Standards And Protocols

| Boundary | Profile |
|---|---|
| SurrealDB `3.3.0` | The `surreal` binary from the server image pinned by OCI digest, one Tokio child process per fixture on a loopback ephemeral port with a fixture-owned data directory and generated credentials; WebSocket Rust client; complete platform migrations |
| Linux cgroup v2 | Optional storage measurements read the fixture process cgroup's `io.stat` write counters, keyed by block-device major/minor |
| Rust test harness | Source module reused by owning integration tests; no separate smoke process or assertion framework |

`store.rs` owns one disposable database and two independent database-editor clients.
It generates ephemeral credentials, applies current kernel and explicitly selected
owner lanes as fixture admin,
and transfers cleanup ownership only after setup succeeds. Dropping the fixture force-removes
its named container, including the container's writable layer. `new` selects memory;
`with_backend(StoreBackend::RocksDb)` selects an owned `/tmp` database with a 64 MiB
block cache and two 16 MiB write buffers. Both profiles use the same image, owner-lane installer
and database-editor clients. RocksDB qualification uses the installation's storage
engine without creating a host volume. The fixture owns no data that needs a graceful flush;
this avoids waiting for server shutdown while the fixture still holds client sockets. Computers admission and shared Task recovery use the same setup,
which avoids maintaining divergent copies of database lifecycle and cleanup code.
The fixture never connects to the installation database. It proves store behavior,
not public deployment or provider execution.

`with_backend_and_schema` applies an owning test's additional schema as fixture
admin before creating runtime clients. Its ten-second deadline covers that setup.
This permits sequence and schema measurements while keeping test operations on the
database-editor role used by services.

## Catalog Registration

`catalog_registry.rs` reuses the production registration source in
[`platform/gateway/catalog`](../../platform/gateway/catalog/DESIGN.md). Tests therefore
exercise the same kernel and owner declarations as installation binaries without
maintaining another registration list. Source reuse lets an owner test use its own
library types without a reverse package dependency on the installation recipe.

`registry()` clones one binding within the test executable. `fresh_registry()` creates
an independent binding for rejection tests: admitted handles and targets from one
registry cannot authorize requests through another. `catalog_admission.rs` supplies
the gateway admission wrapper over these declarations. Domain assertions stay in
the owning suites; the fixtures provide no policy decisions or installation credentials.

## Lifecycle Bounds

`store/server.rs` runs one `surreal` process per fixture on a loopback port, with memory
storage or RocksDB in a fixture-owned temporary directory. Development environments
install the SurrealDB 3.3.0 binary from the image that `veoveo-deploy` pins, so tests
need no container runtime. A missing binary fails the fixture with that requirement
in the diagnostic. Startup has 30 seconds and fails early when the process exits.
Diagnostics identify the stage, fixture name and exit code or I/O category, and omit
arguments, environment and child output.

The guard transfers to `TestDb` after setup succeeds. Restart stops the process and
starts it on the same port and data directory. Drop kills the process and removes the
directory.

Migration/readiness has 60 seconds. Runtime credential creation and each client
connection have ten seconds. Domain tests can put setup before their SQL assertion
deadline, as Reason's index tests do.

## Storage Measurements

`store/io.rs` reads write bytes and operation counts from the cgroup that contains the
fixture's server process, and `storage_bytes` reports the size of its data directory.
Inside an environment that cgroup holds the whole sandbox, so the ignored measurement
tests are not qualified there. The I/O probe preserves separate block-device counters
because summing stacked devices can double-count writes.
[Linux documents the accounting profile](https://docs.kernel.org/admin-guide/cgroup-v2.html).

The owning workload invokes `compact` before and after its timed writes. The helper
connects with the isolated fixture-admin credentials and gives
[`ALTER DATABASE COMPACT`](https://surrealdb.com/docs/reference/query-language/statements/alter/database)
30 seconds to finish. Runtime writes and replay continue through database-editor
clients. A host `sync` followed by two seconds of stable counters drains kernel
writeback; that observation has a 30-second deadline. Compaction and writeback settling are
outside the latency window.

The difference measures cgroup-attributed block writes for the workload plus requested
compaction. It does not establish steady-state write amplification or SSD NAND writes.

## Input Admission

`tool_inputs.rs` shares pure owner-owned input fixtures between contract consumers
and authenticated hosted tests. It checks fixture integrity, decodes serialized
bytes, and mutates controlled objects, required fields and variant tags. Owners
select their actual request types and assert domain state; the helper owns no
transport, lifecycle, provider or domain catalog.

## Platform Selection

`platform-selection/gpu_scheduling.rs` constructs the existing deployment presets
for native chart tests. Full selection includes the required typed NVIDIA allocator
declarations and seven exclusive workload placements. The fixture reuses the deploy
contract's qualified pin declarations and leaves its admission rules intact.
Deployment-runtime chart tests and deployment-smoke Helm tests share this topology;
each suite owns its assertions. The fixture installs no allocator, starts no workload
and supplies no hardware qualification.
