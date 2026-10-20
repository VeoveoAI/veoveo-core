-- veoveo: all tenants (a server's workers address the Tasks they run by ID, whichever tenant owns them)
-- The owning worker records an operation before its dispatch, or reads the operation an
-- earlier attempt recorded. The Task's row stays locked until the statement commits, so
-- the instance keeps the Task between the ownership check and the write.
WITH parent AS (
    SELECT id, server
    FROM tasks.task
    WHERE id = $1 AND server = $2 AND deployment = $14 AND owner_instance = $3
        AND status IN ('running', 'waiting', 'cancel_requested')
    FOR SHARE
),
dispatched AS (
    INSERT INTO tasks.task_operation (
        task_id, operation_key, operation_id, server, provider, completion, check_first_ms,
        check_interval_ms, check_backoff, check_limit, deadline, callback_digest
    )
    SELECT id, $4, $5, server, $6, $7, $8, $9, $10, $11, $12, $13 FROM parent
    ON CONFLICT (task_id, operation_key) DO NOTHING
    RETURNING
        task_id, operation_key, operation_id, provider, external_id, state, completion,
        check_first_ms, check_interval_ms, check_backoff, check_limit, checks_made,
        next_check_at, deadline, outcome, dispatched_at, submitted_at, settled_at,
        unresolved_at, cancel_started_at, cancel_receipt
)
SELECT
    task_id AS "task_id!",
    operation_key AS "operation_key!",
    operation_id AS "operation_id!: StepOperationId",
    provider AS "provider!",
    external_id,
    state AS "state!",
    completion AS "completion!",
    check_first_ms,
    check_interval_ms,
    check_backoff,
    check_limit,
    checks_made AS "checks_made!",
    next_check_at,
    deadline AS "deadline!",
    outcome,
    dispatched_at AS "dispatched_at!",
    submitted_at,
    settled_at,
    unresolved_at,
    cancel_started_at,
    cancel_receipt,
    true AS "first!"
FROM dispatched
UNION ALL
SELECT
    o.task_id, o.operation_key, o.operation_id, o.provider, o.external_id, o.state,
    o.completion, o.check_first_ms, o.check_interval_ms, o.check_backoff, o.check_limit,
    o.checks_made, o.next_check_at, o.deadline, o.outcome, o.dispatched_at, o.submitted_at,
    o.settled_at, o.unresolved_at, o.cancel_started_at, o.cancel_receipt, false
FROM tasks.task_operation AS o
JOIN parent ON parent.id = o.task_id
WHERE o.operation_key = $4
