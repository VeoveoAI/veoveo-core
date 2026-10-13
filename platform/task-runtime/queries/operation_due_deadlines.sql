-- veoveo: all tenants (the due pass settles the deadlines of the server's operations of every tenant)
-- Mark up to $2 of server $1's pending operations whose deadline passed as unresolved,
-- earliest first, and queue each Task parked on one. An unresolved operation still
-- settles when a final event arrives later.
WITH due AS (
    SELECT task_id, operation_key
    FROM tasks.task_operation
    WHERE server = $1 AND deadline <= now() AND state IN ('dispatching', 'submitted')
    ORDER BY deadline
    LIMIT $2
    FOR UPDATE SKIP LOCKED
),
unresolved AS (
    UPDATE tasks.task_operation AS o
    SET state = 'unresolved', unresolved_at = now(), next_check_at = NULL
    FROM due
    WHERE o.task_id = due.task_id AND o.operation_key = due.operation_key
    RETURNING o.task_id, o.operation_key
),
woken AS (
    UPDATE tasks.task AS task
    SET status = 'queued', status_message = 'Queued', parked_on = NULL
    FROM unresolved
    WHERE task.id = unresolved.task_id AND task.parked_on = unresolved.operation_key
        AND task.status = 'waiting'
    RETURNING task.id
)
SELECT (SELECT count(*) FROM unresolved) AS "unresolved!", (SELECT count(*) FROM woken) AS "woken!"
