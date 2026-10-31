-- veoveo: all tenants (a provider names an operation of the server by its own identifier, whichever tenant owns the Task)
-- Store one provider event of an operation, found by its operation identity, with the
-- callback token's digest when the event came through a callback URL, in one
-- transaction. The operation's row lock orders concurrent deliveries: the first final
-- event settles the operation, and a final event after it is stored as late. A repeated
-- event identifier stores nothing. The settled operation's Task and key return for
-- operation_wake.sql, the transaction's next statement, which queues a Task parked on it.
WITH op AS (
    SELECT task_id, operation_key, server, provider, state
    FROM tasks.task_operation AS operation
    WHERE operation_id = $1 AND server = $2 AND ($3::bytea IS NULL OR callback_digest = $3)
    AND EXISTS (SELECT 1 FROM tasks.task AS affinity WHERE affinity.id = operation.task_id AND affinity.deployment = $7)
    FOR UPDATE
),
event AS (
    INSERT INTO tasks.task_operation_event (
        server, provider, event_id, task_id, operation_key, payload, final, applied
    )
    SELECT
        server, provider, $4, task_id, operation_key, $5, $6,
        $6 AND state IN ('dispatching', 'submitted', 'unresolved')
    FROM op
    ON CONFLICT (server, provider, event_id) DO NOTHING
    RETURNING task_id, operation_key, applied
),
settled AS (
    UPDATE tasks.task_operation AS o
    SET state = 'settled', outcome = $5, settled_at = now(), next_check_at = NULL
    FROM event
    WHERE event.applied AND o.task_id = event.task_id
        AND o.operation_key = event.operation_key
    RETURNING o.task_id, o.operation_key
)
SELECT
    (SELECT count(*) FROM op) AS "found!",
    (SELECT count(*) FROM event) AS "stored!",
    (SELECT count(*) FROM settled) AS "settled!",
    (SELECT task_id FROM settled) AS "settled_task",
    (SELECT operation_key FROM settled) AS "settled_key",
    (
        SELECT task.status IN ('succeeded', 'failed', 'cancelled')
        FROM tasks.task AS task
        JOIN op ON op.task_id = task.id
    ) AS "finished"
