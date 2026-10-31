# Durable Task Runtime

`veoveo-task` stores the Tasks of registered servers in the installation's
PostgreSQL database and runs them on the servers' process instances. A server creates a
Task for a caller's long-running request. One of its processes claims and runs the
Task, and the caller reads, answers, cancels and watches it through `OwnerTaskQuery`.
`veoveo-hub-sdk` adapts it to MCP Tasks, and the runtime itself names no protocol.

## Standards And Protocols

| Standard or interface | Profile |
|---|---|
| [PostgreSQL 18](https://www.postgresql.org/docs/18/) | The `tasks` schema of the [database design](../../docs/DATABASE.md), migrations `0400`–`0401` in `platform/store/migrations`; every query is a `.sql` file in `queries/` checked by sqlx 0.9.0 |
| `SELECT … FOR UPDATE SKIP LOCKED` | Work claims |
| `LISTEN` / `NOTIFY` | `kernel.notify_change` payloads, keyed by Task ID and tenant, on the channel of the Task's server, `tasks.<server>` |
| Row-level security | A member of `veoveo_server` reads and writes only the Tasks of its own servers |
| RFC 9562 UUIDv7 | Native Task identity, where lookups reject other versions, and the operation identity of a Task step or operation |
| RFC 4648 base64url, FIPS 180-4 SHA-256 | Callback tokens: 256 random bits in 43 unpadded base64url characters, stored only as their SHA-256 |

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
Hub profile a call arrived on, and its owner reads the Task only through the same
surface. An input's kind names what the Task asks for, as the adapter that relays the
request spells it, such as `elicitation/create`. The runtime stores both without
interpreting them, so the schema holds no protocol vocabulary.

A unique index on logical server, deployment, tenant, principal, surface and idempotency key returns the
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

The runtime attaches its required `DeploymentId` at creation; `CreateTask` cannot
choose another deployment. The immutable snapshot retains it. Reads, owner pages,
claims, input/step/operation access and worker writes select the same deployment in
SQL, in addition to the existing caller and owner-instance checks. Idempotency is
independent across deployments sharing a logical server. A replacement process with
the same deployment may recover resumable Tasks; a different deployment cannot read
or claim them. `interrupted_indeterminate` still fails without reexecution.

This is execution affinity, not schema cloning or a new database authorization
boundary: existing logical-server roles and admitted shared data remain unchanged.
Notifications remain logical-server wake hints; consumers reread under deployment
selection. Retention and due passes may maintain the logical server's rows across
deployments, but they never execute provider work, change deployment identity or
transfer a claim. A queued due Task remains consumable only by its own deployment.
Provider callback URL/routing contracts remain the adapter's responsibility.

`TaskRuntime::connect` opens the store and starts the process's `Instance`;
`TaskRuntime::new` takes a store, a logical server, a required `DeploymentId` and an instance the caller started. A claim
records the instance in `owner_instance`, which references `kernel.instance` with
`ON DELETE SET NULL`. When the process ends, its instance lock is released; once the
lock has stayed free for the instance grace, a reap deletes the instance row, and the
cascade clears `owner_instance` on its Tasks.
`task_recovery` then moves each unfinished Task to the state its recovery class
declares:

| Released Task | Becomes |
|---|---|
| Cancel requested | `cancelled` |
| `resume` | `queued`, with progress and start time cleared, so another instance runs it again |
| `interrupted_indeterminate` | `failed` with code `interrupted_indeterminate`, because its effect may have partly happened |

Every worker write requires the caller's instance as owner. A process whose instance
was reaped settles nothing and receives `TaskError::NotOwner`. `release` gives up a
claim without an outcome, which moves the Task on exactly as the end of its instance
would.

A process whose instance connection fails keeps running: the instance
[reconnects](../../docs/DATABASE.md#process-instances) and keeps its Tasks while no
reaper deleted its row, which no reaper does before the lock has stayed free for the
instance grace. `TaskRuntimeConfig::instance_grace` sets the grace, 30 seconds by
default. Once a reaper deleted the row, the instance continues as a new generation
under a new id. Before its next claim the runtime aborts the local workers of earlier
generations that still run and waits for them, so a Task the reaper returned to the
queue never runs twice in one process; their writes, made under the new id, match no
row in any case.

`register_worker` tracks a worker until its task ends, whether it returns, panics or is
aborted, so the runtime holds only workers that still run and `local_workers` counts
them. A Task that settles while its worker still runs keeps the entry until the worker
returns, so a stop still reaches it.
An instance that ended, because it shut down or the database refused its login, stops
claiming.

## Work

`TaskRuntime::work` returns a stream of Tasks this instance claimed. It claims the
oldest queued Task of its server and deployment, or a parked Task whose cancellation awaits its
provider, for each polled item with `FOR UPDATE SKIP LOCKED`, so
replicas sharing the queue take different Tasks without waiting. With the queue empty,
the stream waits for a created or changed Task of its server, a reap round of its
instance or a listener reconnect, and then claims again. Another server's Tasks never
wake it. A Task returned to the queue by any reaper
therefore reaches a live worker. A claim that fails, as it does while the database
connection is down, is tried again after 100 milliseconds, doubling to at most five
seconds between attempts. The stream ends with an error only when the instance ended.
`claim` takes one queued Task by ID, and `claim_next` takes the oldest.

## Transitions

A worker settles its Task with `transition` or `transition_if_current`. The statement
requires the caller's instance as owner and the snapshot's `revision`, so work derived
from an older snapshot fails with `Conflict`. Worker transitions start from running,
waiting or cancel-requested Tasks. A terminal status records the completion time and
clears the owner. `ensure_owner` lets a worker check ownership at its own checkpoints.

`transition_resumable` and `transition_resumable_if_current` settle `resume` Tasks. A
pending cancellation turns the outcome into `Cancelled`, except what the
`ResumeCancellationPolicy` keeps. `CancellationWins` keeps nothing, `PreserveFailure`
keeps a genuine failure, and `PreserveOutcome` keeps a success or a genuine failure,
because a request that arrives after the work finished does not undo it. Only under
`PreserveOutcome` may a success follow `cancel_requested`; a plain worker transition
still cannot. Unfinished work, such as a progress update, is cancelled under every
policy. A rejected write
rereads the Task once. A finished Task is returned as it is, a cancellation request is
settled, and any other change fails. While a stop token is cancelled, a success is
withheld and the unchanged Task is returned, unless cancellation is pending, which is
settled.

`TaskRuntime::cancel` and `OwnerTaskQuery::cancel` request cancellation in one statement.
An unowned Task has no worker to observe the request and is cancelled at once, except a
Task parked on an operation, which a claim takes to carry the cancellation to the
provider. An owned
Task becomes `cancel_requested`, and its worker settles it. A local worker registered with
`register_worker` is stopped through its token. A finished Task is returned unchanged.

## Caller Access

`TaskRuntime::for_owner` builds an `OwnerTaskQuery` for one `TaskOwner`. Every query
selects in SQL by server, runtime deployment, tenant, principal, surface and explicit tenant claim, and
requires the caller to hold every data label the owner held. `of_type` and `of_types`
select 1–32 operations. `in_work_context` adds the caller's Work Context after checking
that it belongs to the owner's tenant. A missing Task and another caller's Task both read
as `None`.

`get` reads one Task. `page` reads up to 1,000 in a `TaskPageOrder`, oldest first or
newest first by creation time and then ID, after a `TaskPageCursor`. A cursor records
the position of the last Task and the order of the page that issued it, and a page in
the other order refuses it with `PageOrderMismatch`. The position bounds the
`task_owner_page` index range in generic plans in both orders: a newest-first page
scans the same index backward. `cancel` requests cancellation and
`submit_input_responses` answers inputs.

`TaskRuntime::get` and `TaskRuntime::owner` read a Task of the runtime's server without
a caller selection. `TaskRuntime::get_many` reads up to 1,000 of the server's Tasks by ID
in one query and returns them in ID order. It leaves out a missing ID and another
server's Task, counts a repeated ID once, and refuses a larger batch with
`BatchTooLarge` before it reads. A server uses it to reconcile its own records with the
Tasks behind them, such as removing records whose Task retention has pruned.

Row-level security adds a second boundary. A registered server's login is a member of
`veoveo_server`, and the policies on the three tables admit only rows whose server is in
`kernel.current_servers()`. An input or a step belongs to the server of its Task. The
installation owner owns the tables, so the policies do not apply to the Hub.

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

## Operations

An operation is outside work a Task hands to a provider: a generation, a solver run, a
virtual machine to provision. The worker records it before it calls the provider, parks
the Task while the outcome is pending, and resumes when the outcome arrives. A parked
Task holds no worker and no connection, and a restart leaves it waiting.

```rust
let spec = OperationSpec::events(OperationKey::parse("generate")?, provider, hours(6));
let job = runtime
    .dispatch_operation(task.task_id, &spec, |attempt| async move {
        match attempt.dispatch {
            StepDispatch::First => provider.submit(&input, attempt.operation_id, attempt.callback).await,
            // An earlier attempt may have reached the provider: reconcile, never resubmit.
            StepDispatch::Unsettled { .. } => provider.find(attempt.operation_id).await,
        }
    })
    .await?;
match runtime.wait_operation::<Prediction, _>(task.task_id, &spec.key, &calls).await? {
    WaitOutcome::Settled(prediction) => store_outputs(prediction).await?,
    WaitOutcome::Expired => mark_unknown().await?,
    WaitOutcome::Cancelled { receipt } => settle_cancelled(receipt).await?,
}
```

| Table | Holds |
|---|---|
| `tasks.task_operation` | One row per Task and `OperationKey`: the UUIDv7 operation ID the provider call uses as its idempotency key, the `ProviderName` the server chose, the provider's `ExternalJobId` once submitted, the state, how the outcome arrives, the check schedule and count, the deadline, the SHA-256 of the callback token, the outcome once settled and the cancellation's start and receipt |
| `tasks.task_operation_event` | Each event the provider reported, by server, provider and event ID: the operation, the payload, whether it was final and whether it settled the operation |

`dispatch_operation` records the operation in the statement that checks the worker's
ownership, then calls the closure with an `OperationAttempt`. The first attempt receives
a fresh `CallbackToken` for the provider's callback URL; the runtime keeps only its
SHA-256. A successful closure records its `Submission` and the operation becomes
`submitted`. An operation an earlier attempt recorded without a submission reaches the
closure as `StepDispatch::Unsettled`, with the same operation ID and no token, because
the provider may already hold the first one. A dispatch that finds its operation
submitted or settled returns it without calling the closure.

An operation's outcome arrives in one of two ways, which `Completion` declares:

| Completion | How the outcome arrives |
|---|---|
| `Events` | The server's webhook handler checks the provider's signature and calls `deliver` with an `EventTarget`: the callback URL's operation ID and token, the operation ID alone, or the provider's job ID |
| `Checks` | The wait calls the server's `OperationCalls::check` on a `CheckSchedule`: the first check after a delay, then at an interval that grows by the backoff, at most the schedule's limit of checks |

`deliver` stores the event and settles the operation in one transaction, under the
operation's row lock. The first final event settles a pending or unresolved operation and
queues the Task parked on it; a repeated event ID stores nothing; a final event after the
operation settled, or after its Task finished, is stored as late. The Task is queued by
the transaction's second statement, after the lock is granted. PostgreSQL's
read-committed statements read rows as they were when the statement began, so the
statement that waited for the lock would find a Task whose park held that lock as still
running and leave it parked on a settled operation, with nothing left to queue it. Any replica delivers,
and the server acknowledges the provider only after `deliver` returns. `Delivery::Unknown`
answers an event that names no operation of the server or carries the wrong token.

`wait_operation` returns a settled operation's outcome decoded as the caller's type.
Otherwise it parks the Task: one statement locks the operation, confirms it is still
pending with no check due, and moves the Task to `waiting` with no owner and
`parked_on` naming the operation, then the wait fails with `TaskError::Parked`. The
worker propagates it and stops without settling the Task. The recovery trigger skips a
Task parked on an operation, so a reaped instance changes nothing. A delivery queues the
Task, and its next run replays its finished steps and its dispatch from the database and
reaches the wait again, which returns the outcome.

Every runtime runs a due pass. It queues Tasks parked on an operation whose check is due,
marks pending operations whose deadline passed as `unresolved` and, in the same
transaction's next statement as a delivery does, queues the Tasks parked on them, in
batches with `SKIP LOCKED`, then sleeps until the server's next due
time, at most `DUE_INTERVAL`, five seconds. A resumed wait on a checked operation runs the
check and records it as an event. A wait on an unresolved operation returns
`WaitOutcome::Expired`: the outcome is unknown, which never counts as a failure, and a
later final event still settles the operation.

A cancellation of a parked Task leaves it `cancel_requested` and unowned instead of
cancelling it at once, because the provider may still be working. Claims take such a
Task without moving it to running. Its wait records that the cancellation started, calls
`OperationCalls::cancel`, stores the provider's receipt and returns
`WaitOutcome::Cancelled`; the worker then settles the Task as cancelled. A process that
ends during the cancel call leaves the cancellation started without a receipt, and
recovery cancels the Task. Events the provider sends after that are stored as late.

The operation table keeps each operation's identity, provider, completion profile and
dispatch time, and a settled operation its outcome: `task_operation_settle` rejects any
other change, and events never change. Retention never prunes a Task with an operation
that has not settled, so a late event still finds it; an operator releases such a Task by
settling or deleting its operation.

## Retention

Creation sets `retention_expires_at` from the TTL, seven days by default, and rejects a
deadline after year 9999. A creation may add retention pins, and
`acknowledge_retention_pin` removes one. `prune_expired` deletes the server's finished
Tasks whose retention expired, that hold no pin and whose operations have settled, with
their inputs, steps and operations, in the store's
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
each class and fencing after an instance's session is terminated and reaped, claims,
wakeups and due checks that continue after every session of a server's login was
terminated, an instance whose lock stayed free for less than the grace keeping its
Tasks, one that stayed away longer losing them to recovery and taking them back as a new
generation, a crashed process's Tasks taken by another process only after the grace,
the workers of an earlier instance generation stopped before the new
generation claims their Task again, worker entries that leave once the worker returns,
panics or is aborted, a work stream that ends once the server's login is
dropped, releases into each recovery class,
cancellation of owned and unowned Tasks, resumable settlement, input exchanges with
concurrent answers and concurrent requests past the input limit, steps recorded,
returned without rerunning and left unsettled across a failed attempt and an instance's
death, step fencing and the step table's constraints, operations parked across an instance's
death and resumed by another instance with their steps replayed, repeated and late
events, concurrent final events of which exactly one settles, check schedules with their
backoff and limit, deadlines that leave the outcome open and keep the Task from
retention until a late event, cancellation carried to the provider with its receipt,
unsettled dispatches reported with the same operation ID, idempotent creation,
result and failure shapes, owner reads and pages, subscriptions across a listener
reconnect, row-level security between two server logins over Tasks, steps and operations, and
batched retention pruning with the steps it removes.
Unit tests on the same fixture check that another server's Tasks never wake a runtime's
listener and that a runtime prunes expired Tasks on its own schedule.
`tests/database_conformance.rs` holds every query under `queries` and the `tasks`
schema to the [server schema rules](../../docs/DATABASE.md#server-schema-rules). Worker
statements address Tasks by ID across the server's tenants and say so in their markers.
