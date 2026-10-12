# Shared Native Test Fixtures

## Standards And Protocols

| Boundary | Profile |
|---|---|
| Rust test harness | Source modules reused by owning tests; no separate smoke process or assertion framework |

## Catalog Registration

`catalog_registry.rs` reuses the production registration source in
[`platform/gateway/catalog`](https://github.com/BiomaAI/veoveo/blob/0c33cf8a7e5a1d46f953702553ac6643f99c1363/platform/gateway/catalog/DESIGN.md). Tests therefore
exercise the same kernel and owner declarations as installation binaries without
maintaining another registration list. Source reuse lets an owner test use its own
library types without a reverse package dependency on the installation recipe.

`registry()` clones one binding within the test executable. `fresh_registry()` creates
an independent binding for rejection tests: admitted handles and targets from one
registry cannot authorize requests through another. `catalog_admission.rs` supplies
the gateway admission wrapper over these declarations. Domain assertions stay in
the owning suites; the fixtures provide no policy decisions or installation credentials.

## Input Admission

`tool_inputs.rs` shares pure owner-owned input fixtures between contract consumers
and authenticated hosted tests. It checks fixture integrity, decodes serialized
bytes, and mutates controlled objects, required fields and variant tags. Owners
select their actual request types and assert domain state; the helper owns no
transport, lifecycle, provider or domain catalog.
