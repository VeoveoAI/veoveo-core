-- One operation of this server's Task.
SELECT
    task_id, operation_key, operation_id AS "operation_id: StepOperationId", provider,
    external_id, state, completion, check_first_ms, check_interval_ms, check_backoff,
    check_limit, checks_made, next_check_at, deadline, outcome, dispatched_at, submitted_at,
    settled_at, unresolved_at, cancel_started_at, cancel_receipt
FROM tasks.task_operation
WHERE task_id = $1 AND operation_key = $2 AND server = $3
