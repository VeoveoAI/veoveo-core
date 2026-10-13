-- veoveo: all tenants (a server's workers address the Tasks they run by ID, whichever tenant owns them)
-- The owning worker records one status check of a checked operation. A final result
-- settles the operation; any other result schedules the next check with the operation's
-- backoff, until its check limit or its deadline.
WITH parent AS (
    SELECT id
    FROM tasks.task
    WHERE id = $1 AND server = $2 AND owner_instance = $3
        AND status IN ('running', 'waiting', 'cancel_requested')
    FOR SHARE
),
op AS (
    SELECT o.task_id, o.operation_key, o.server, o.provider
    FROM tasks.task_operation AS o
    JOIN parent ON parent.id = o.task_id
    WHERE o.operation_key = $4 AND o.completion = 'checks'
        AND o.state IN ('dispatching', 'submitted')
    FOR UPDATE OF o
),
event AS (
    INSERT INTO tasks.task_operation_event (
        server, provider, event_id, task_id, operation_key, payload, final, applied
    )
    SELECT server, provider, $5, task_id, operation_key, $6, $7, $7 FROM op
    ON CONFLICT (server, provider, event_id) DO NOTHING
    RETURNING task_id, operation_key
)
UPDATE tasks.task_operation AS o
SET checks_made = o.checks_made + 1,
    state = CASE WHEN $7 THEN 'settled' ELSE o.state END,
    outcome = CASE WHEN $7 THEN $6 END,
    settled_at = CASE WHEN $7 THEN now() END,
    next_check_at = CASE
        WHEN $7 OR o.checks_made + 1 >= o.check_limit THEN NULL
        ELSE least(
            now() + make_interval(
                secs => o.check_interval_ms * power(o.check_backoff, least(o.checks_made, 64))
                    / 1000.0
            ),
            o.deadline
        )
    END
FROM event
WHERE o.task_id = event.task_id AND o.operation_key = event.operation_key
RETURNING
    o.task_id, o.operation_key, o.operation_id AS "operation_id: StepOperationId", o.provider,
    o.external_id, o.state, o.completion, o.check_first_ms, o.check_interval_ms,
    o.check_backoff, o.check_limit, o.checks_made, o.next_check_at, o.deadline, o.outcome,
    o.dispatched_at, o.submitted_at, o.settled_at, o.unresolved_at, o.cancel_started_at,
    o.cancel_receipt
