# Usage Ledger

`veoveo-usage` records what servers metered for whom: a generation, tokens, solver
hours, stored bytes. Each server declares its own kinds of usage and appends an estimate
before billed work and the actual amount once the provider reports it, inside the same
transaction as the work. Readers total a Task's cost and a tenant's days.
`veoveo-usage-contract` holds the record types without a database driver. The ledger
names no server, unit vocabulary beyond FOCUS's format, or price.

## Standards And Protocols

| Standard or interface | Profile |
|---|---|
| [FinOps Open Cost and Usage Specification (FOCUS) 1.4](https://focus.finops.org/focus-specification/) | Record columns, ServiceCategory and ChargeCategory values, ChargeClass `Correction`, and the Unit Format of quantities; a FOCUS export is a later step, and the mapping below fixes its columns |
| ISO 4217 | Alphabetic currency codes; `XXX` marks a stored record without money |
| [PostgreSQL 18](https://www.postgresql.org/docs/18/) | The `usage` schema, migration `0600_usage.sql` in `platform/store/migrations`, with records partitioned by month; every query is a `.sql` file in `queries/` checked by sqlx 0.9.0 |
| Row-level security | A member of `veoveo_server` reads only the records and kinds of its own servers, and writes only through the SECURITY DEFINER functions |
| RFC 9562 UUIDv7 | Record identity |
| RFC 3339 | Charge period timestamps |

## Kinds

A `UsageKind` is `<owner>.<name>`, such as `media.generation`, where the owner is the
declaring server's slug and the name a snake_case noun. A `UsageKindDeclaration` gives
the kind's FOCUS `ServiceCategory`, the FOCUS unit its quantities count, such as `Units`,
`Tokens` or `GB-Hours`, and whether its records carry money.

`UsageStore::declare_as_server` declares a server's kinds through its own login with
`usage.declare_server_kinds`. That function admits a kind only under the prefix of a
server whose group role the login holds and records that server; a kind outside the
prefix, or one another owner declared, fails as `UsageStoreError::ForeignKind`. The
installation owner declares its own kinds with `UsageStore::declare`. Repeating a
declaration changes nothing.

## Records

A `UsageDraft` carries the kind, the phase, the caller the work ran for, the quantity,
a source identifier and the charge period, and optionally the Task and operation, the
server and Task that did the work for the caller through the Hub, money, a FOCUS
charge category, the correction flag and the target resource's URI. Quantities and
amounts are exact decimals written as text, so no value passes through a binary float.

```rust
let estimate = UsageDraft::builder(
    UsageKind::parse("media.generation")?,
    UsagePhase::Estimate,
    actor,
    Quantity::whole(1),
    UsageSourceId::parse(format!("task:{task}:generate"))?,
    ChargePeriod::at(now),
)
.task(task, Some("generate"))
.money(Money { amount: Amount::parse("0.04")?, currency: Currency::parse("USD")? })
.build()?;
UsageWrite::reserve(&mut transaction, &estimate).await?;
```

`UsageWrite::append_as_server` appends in the caller's transaction through
`usage.append_server_records`, which admits only kinds the login's servers declared and
then runs `usage.append`. A record's source is the owner of its kind, so a server writes
only its own usage. A record carries money exactly when its kind does. The source,
kind, phase and source identifier form the record's idempotency key: a repeated draft
writes nothing, and one that disagrees with the stored record fails as
`UsageStoreError::Conflict`. `UsageWrite::reserve` appends an estimate before billed
work; installation budgets refuse a reservation there, before the provider is called.

Records never change. A refund is a `Credit` record and a correction a record with the
correction flag. Each new record adds to its day's totals in `usage.daily` in the same
transaction, so a rolled-back append leaves the totals as they were.

## Reads

`UsageStore::task_usage` returns a Task's cost per kind and currency: its actual records
of a kind when it has any, else its estimates of that kind. `UsageStore::daily` returns a
tenant's totals per day, principal, kind, phase and currency. Both filter the tenant in
SQL, and a server's login reads only its own servers' rows.

## Storage

| Relation | Holds |
|---|---|
| `usage.kind` | Each declared kind with its category, unit, money flag, declaration time and declaring server |
| `usage.record` | One row per record, partitioned by the month its charge period starts, with a default partition for months without one |
| `usage.record_key` | Each record's idempotency key, because a partitioned table enforces uniqueness only within a partition |
| `usage.daily` | Totals per tenant, day, principal, source, kind, phase and currency |

`UsageStore::prepare_partitions` creates the monthly partitions from the current month
through a given month, as the installation owner. It skips a month whose records already
sit in the default partition. The installation owner prepares months ahead and drops a
month's partition when its retention passes.

## FOCUS Columns

| FOCUS 1.4 column | Ledger value |
|---|---|
| `ChargePeriodStart`, `ChargePeriodEnd` | The charge period |
| `ServiceName` | The source server |
| `ServiceCategory` | The kind's declared category |
| `SkuId`, `ChargeDescription` | The kind |
| `ConsumedQuantity`, `ConsumedUnit` | The quantity and the kind's unit |
| `BilledCost`, `EffectiveCost`, `BillingCurrency` | The amount and currency of actual records |
| `ChargeCategory`, `ChargeClass` | The charge category, and `Correction` for a correction |
| `BillingAccountId`, `SubAccountId` | The installation and the tenant |
| `ResourceId` | The target URI |
| `x_Principal`, `x_WorkContext`, `x_Task`, `x_ActingServer`, `x_ActingTask`, `x_Phase` | Custom columns for the caller, the Work Context, the Task, the server and Task that did the work for the caller, and estimates, which FOCUS does not describe |

## Verification

`tests/ledger.rs` runs against a fresh database from the
[PostgreSQL fixture](../../testing/fixtures/DESIGN.md): confined declarations and
appends, idempotent and conflicting repeats, a Task's cost from actuals where present and
estimates otherwise, daily totals including a rolled-back append, concurrent repeats
that store one record, money that must follow the kind, row-level security between two
server logins, and records routed to their month's partition.
`tests/database_conformance.rs` holds the queries and the `usage` schema to the
[server schema rules](../../docs/DATABASE.md#server-schema-rules).
