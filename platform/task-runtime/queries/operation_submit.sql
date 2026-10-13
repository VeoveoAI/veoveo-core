-- veoveo: all tenants (a server's workers address the Tasks they run by ID, whichever tenant owns them)
-- The owning worker records that its dispatch reached the provider, with the provider's
-- job identifier when it has one. A checked operation schedules its first check.
WITH parent AS (
    SELECT id
    FROM tasks.task
    WHERE id = $1 AND server = $2 AND owner_instance = $3
        AND status IN ('running', 'waiting', 'cancel_requested')
    FOR SHARE
)
UPDATE tasks.task_operation AS o
SET state = 'submitted',
    external_id = $6,
    submitted_at = now(),
    next_check_at = CASE
        WHEN o.completion = 'checks'
            THEN least(now() + o.check_first_ms * interval '1 millisecond', o.deadline)
    END
FROM parent
WHERE o.task_id = parent.id AND o.operation_key = $4 AND o.operation_id = $5
    AND o.state = 'dispatching'
RETURNING
    o.task_id, o.operation_key, o.operation_id AS "operation_id: StepOperationId", o.provider,
    o.external_id, o.state, o.completion, o.check_first_ms, o.check_interval_ms,
    o.check_backoff, o.check_limit, o.checks_made, o.next_check_at, o.deadline, o.outcome,
    o.dispatched_at, o.submitted_at, o.settled_at, o.unresolved_at, o.cancel_started_at,
    o.cancel_receipt
