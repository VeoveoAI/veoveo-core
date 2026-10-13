# Durable Task Runtime

`veoveo-task-runtime` stores the Tasks of registered servers in the installation's
PostgreSQL database and runs them on the servers' process instances. A server creates a
Task for a caller's long-running request. One of its processes claims and runs the
Task, and the caller reads, answers, cancels and watches it through `OwnerTaskQuery`.
`veoveo-sdk-rust` adapts it to MCP Tasks, and the runtime itself names no protocol.

## Standards And Protocols

| Standard or interface | Profile |
|---|---|
| [PostgreSQL 18](https://www.postgresql.org/docs/18/) | The `tasks` schema of the [database design](../../docs/DATABASE.md), migrations `0400`–`0410` in `platform/store/migrations`; every query is a `.sql` file in `queries/` checked by sqlx 0.9.0 |
| `SELECT … FOR UPDATE SKIP LOCKED` | Work claims |
| `LISTEN` / `NOTIFY` | `kernel.notify_change` payloads, keyed by Task ID and tenant, on the channel of the Task's server, `tasks.<server>` |
| Row-level security | A member of `veoveo_server` reads and writes only the Tasks of its own servers |
| RFC 9562 UUIDv7 | Native Task identity, where lookups reject other versions, and the operation identity of a Task step |

The Task contract's statuses and recovery classes, `resume` and
`interrupted_indeterminate`, are the runtime's own vocabulary. A protocol adapter maps
them onto its protocol's statuses.

## Schema

`tasks.task` holds one row per Task. The columns that queries filter on are typed: the
server, the owner's tenant, explicit tenant claim, principal, kind, issuer, subject and
surface, the owner's data labels, the Work Context, the operation, the status, the
recovery class and the owner instance. `tenant_id` and `principal_id` reference the
owner's rows in `identity`, which creation records through `ensure_identity`, and
`task_tenant_id` and `task_principal_id` index them for the foreign-key checks. JSON
columns hold the request input, the result, failure details and the invocation
authority. SQL `NULL` in a JSON column is an absent value and JSON `null` a present one,
so a Task can succeed with a `null` result.

| Constraint | Rule |
|---|---|
| `task_id_v7` | The ID is an RFC 9562 UUIDv7 |
| `task_authority` | The authority names the row's tenant and Work Context |
| `task_type_name`, `task_status`, `task_recovery_class`, `task_owner_kind` | Vocabularies use their wire spellings |
| `task_result`, `task_result_uri` | Exactly the succeeded Tasks have a result, and only they have a product address |
| `task_error` | Exactly the failed Tasks have a failure code and message |
| `task_completion` | Exactly the finished Tasks have a completion time |
| `task_owner_instance` | Only running, waiting and cancel-requested Tasks have an owner |
| `task_progress`, `task_ttl`, `task_poll_interval` | Progress lies in 0..=1, and durations are non-negative |
| `task_surface`, `task_idempotency_key`, `task_retention_pins` | Surfaces, keys and pins are 1 to 256 bytes without control characters; pins are distinct |

A Task's surface is the opaque `TaskSurface` the creating adapter supplies, such as the
Gateway profile a call arrived on, and its owner reads the Task only through the same
surface. An input's kind names what the Task asks for, as the adapter that relays the
request spells it, such as `elicitation/create`. The runtime stores both without
interpreting them, so the schema holds no protocol vocabulary.

A unique index on server, tenant, principal, surface and idempotency key returns the
existing Task to a repeated creation, and pruning the Task releases its key.
`tasks.task_input` holds the inputs a Task requests and the caller's answers, keyed by
Task and request key. `tasks.task_step` holds the [steps](#steps) of a Task's work,
keyed by Task and step key. The rows of both tables go with their Task.

Three triggers maintain the table. `task_revision` advances `revision` and `updated_at`
on every update and rejects a change to the Task's identity, owner or request.
`task_recovery` applies recovery when a Task loses its owner. `task_notify` reports every
insert, update and delete on the channel of the Task's server, such as `tasks.time`, and
on no other channel.

## Ownership And Recovery

`TaskRuntime::connect` opens the store and starts the process's `Instance`;
`TaskRuntime::new` takes a store, a server and an instance the caller started. A claim
records the instance in `owner_instance`, which references `kernel.instance` with
`ON DELETE SET NULL`. When the process ends, its instance lock is released, the next
reap deletes the instance row, and the cascade clears `owner_instance` on its Tasks.
`task_recovery` then moves each unfinished Task to the state its recovery class
declares:

| Released Task | Becomes |
|---|---|
| Cancel requested | `cancelled` |
| `resume` | `queued`, with progress and start time cleared, so another instance runs it again |
| `interrupted_indeterminate` | `failed` with code `interrupted_indeterminate`, because its effect may have partly happened |

Every worker write requires the caller's instance as owner. A process whose instance
was reaped settles nothing and receives `TaskError::NotOwner`. A lost instance also
stops claiming.

## Work

`TaskRuntime::work` returns a stream of Tasks this instance claimed. It claims the
oldest queued Task of its server for each polled item with `FOR UPDATE SKIP LOCKED`, so
replicas sharing the queue take different Tasks without waiting. With the queue empty,
the stream waits for a created or changed Task of its server, a reap round of its
instance or a listener reconnect, and then claims again. Another server's Tasks never
wake it. A Task returned to the queue by any reaper
therefore reaches a live worker. The stream ends with an error when the instance loses
its lock. `claim` takes one queued Task by ID, and `claim_next` takes the oldest.

## Transitions

A worker settles its Task with `transition` or `transition_if_current`. The statement
requires the caller's instance as owner and the snapshot's `revision`, so work derived
from an older snapshot fails with `Conflict`. Worker transitions start from running,
waiting or cancel-requested Tasks. A terminal status records the completion time and
clears the owner. `ensure_owner` lets a worker check ownership at its own checkpoints.

`transition_resumable` and `transition_resumable_if_current` settle `resume` Tasks. A
pending cancellation turns the outcome into `Cancelled`, except that
`ResumeCancellationPolicy::PreserveFailure` keeps a genuine failure. A rejected write
rereads the Task once. A finished Task is returned as it is, a cancellation request is
settled, and any other change fails. While a stop token is cancelled, a success is
withheld and the unchanged Task is returned, unless cancellation is pending, which is
settled.

`TaskRuntime::cancel` and `OwnerTaskQuery::cancel` request cancellation in one statement.
An unowned Task has no worker to observe the request and is cancelled at once. An owned
Task becomes `cancel_requested`, and its worker settles it. A local worker registered with
`register_worker` is stopped through its token. A finished Task is returned unchanged.

## Caller Access

`TaskRuntime::for_owner` builds an `OwnerTaskQuery` for one `TaskOwner`. Every query
selects in SQL by server, tenant, principal, surface and explicit tenant claim, and
requires the caller to hold every data label the owner held. `of_type` and `of_types`
select 1–32 operations. `in_work_context` adds the caller's Work Context after checking
that it belongs to the owner's tenant. A missing Task and another caller's Task both read
as `None`.

`get` reads one Task, and `page` reads up to 1,000 in creation order after a
`TaskPageCursor`, which records only a position. The cursor's position bounds the
`task_owner_page` index range in generic plans too. `cancel` requests cancellation and
`submit_input_responses` answers inputs. `TaskRuntime::get` and `TaskRuntime::owner`
read a Task of the runtime's server without a caller selection.

Row-level security adds a second boundary. A registered server's login is a member of
`veoveo_server`, and the policies on the three tables admit only rows whose server is in
`kernel.current_servers()`. An input or a step belongs to the server of its Task. The
installation owner owns the tables, so the policies do not apply to the Gateway.

## Subscriptions

`OwnerTaskQuery::subscribe` admits up to 256 Task IDs. It joins the runtime's listener
before it reads the caller's Tasks among them, so no change between that read and the
first notification goes unnoticed. The accepted IDs are those the caller can read. The
stream delivers their current state, then rereads a watched Task under the caller's
selection whenever a notification names it. A listener reconnect rereads every watched
Task, because PostgreSQL does not replay notifications sent while a listener was
disconnected. A reader that falls 1,024 signals behind rereads everything as well. A
Task is delivered only when its revision changed, and intermediate revisions may
coalesce.

One `PlatformStore::listen` connection per runtime serves every subscription and work
stream. It listens on `TaskRuntime::channel`, `tasks.<server>`, starts with the first
reader and closes after the last one leaves.

## Input Exchanges

`request_input` stores an input request and moves the owned Task to waiting in one
statement; a repeated key fails with `DuplicateInputKey`. A Task holds at most
`MAX_TASK_INPUTS`, 64, requests, answered or not. The `task_input_limit` trigger counts
them under the Task's row lock, which the same statement takes first, and a request past
the limit fails with `InputLimit`. Callers answer through
`OwnerTaskQuery::submit_input_responses`. Each answer locks its Task, which must be
queued, running or waiting and selected for the caller, and in the same statement it
writes an unanswered input and advances the Task's revision. An answer to an answered or
unknown key is ignored and leaves the Task unchanged. `outstanding_inputs` lists the
unanswered requests for an adapter's input-required projection.

## Steps

A worker splits a Task's work into named steps with `TaskRuntime::step`, so a Task that
runs again after recovery returns the results its earlier run recorded instead of
repeating the work. A `StepKey` names each step: 1 to 128 bytes without control
characters, the same on every run, with a loop's position in the key when a step
repeats, such as `upload-part-3`.

```rust
let charge: Receipt = runtime
    .step(task.task_id, &StepKey::parse("charge")?, |attempt| async move {
        // The operation ID is the same on every attempt, so the provider deduplicates.
        provider.charge(amount, attempt.operation_id).await
    })
    .await?;
```

Before the closure runs, one statement locks the Task's row, checks that this instance
owns the Task and that it is running, waiting or cancel-requested, and records the step
in `tasks.task_step` with a fresh UUIDv7 `StepOperationId`. A step finished by an earlier
attempt returns its stored result without calling the closure. A step that an earlier
attempt dispatched without recording a result passes the closure a `StepAttempt` whose
dispatch is `StepDispatch::Unsettled`, with the same operation ID and the time of the
first dispatch. Its effect may have happened, so the closure reconciles with the system
it called, or passes the operation ID as that system's idempotency key, before it acts
again.

A successful result is serialized to JSON, decoded back into the closure's type and
stored under the same ownership check, and the decoded value is returned. A value whose
JSON does not decode as it was written fails with `TaskError::Json` and records nothing,
so every later run returns the same value as the first. JSON `null` is a stored result.
An error from the closure records nothing and returns to the caller, which leaves the
step unsettled. A write from an instance that lost the Task fails with `NotOwner`, and
the step stays unsettled for the next owner.

The table keeps each step's operation ID and dispatch time, and a finished step keeps its
result: the `task_step_settle` trigger rejects any other update, and check constraints
admit only UUIDv7 operation IDs and valid keys. A `resume` Task that recovery returns to
the queue keeps its steps, and pruning removes them with their Task.

## Retention

Creation sets `retention_expires_at` from the TTL, seven days by default, and rejects a
deadline after year 9999. A creation may add retention pins, and
`acknowledge_retention_pin` removes one. `prune_expired` deletes the server's finished
Tasks whose retention expired and that hold no pin, with their inputs and steps, in the store's
[batches](../store/DESIGN.md#batched-maintenance), earliest expiry first.

The runtime owns retention. Every runtime prunes its server's expired Tasks every
`RETENTION_INTERVAL`, ten minutes, first one interval after it starts, until its last
clone is dropped or its instance is lost. Replicas run their passes independently, and
`FOR UPDATE SKIP LOCKED` gives each batch's rows to one of them. A failed pass is logged
and the next one runs on schedule.

## Verification

`tests/tasks` runs against a fresh database from the
[PostgreSQL fixture](../../testing/fixtures/DESIGN.md). It covers concurrent claims, the
work stream, notifications that reach only the channel of the Task's server, recovery of
each class and fencing after an instance's session is terminated and reaped,
cancellation of owned and unowned Tasks, resumable settlement, input exchanges with
concurrent answers and concurrent requests past the input limit, steps recorded,
returned without rerunning and left unsettled across a failed attempt and an instance's
death, step fencing and the step table's constraints, idempotent creation,
result and failure shapes, owner reads and pages, subscriptions across a listener
reconnect, row-level security between two server logins over Tasks and steps, and
batched retention pruning with the steps it removes.
Unit tests on the same fixture check that another server's Tasks never wake a runtime's
listener and that a runtime prunes expired Tasks on its own schedule.
`tests/database_conformance.rs` holds every query under `queries` and the `tasks`
schema to the [server schema rules](../../docs/DATABASE.md#server-schema-rules). Worker
statements address Tasks by ID across the server's tenants and say so in their markers.
