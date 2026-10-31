-- Mark up to $2 of server $1's pending operations whose deadline passed as unresolved,
-- earliest first, and return them for operation_wake.sql, the transaction's next
-- statement, which queues each Task parked on one. An unresolved operation still settles
-- when a final event arrives later.
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
)
SELECT task_id, operation_key FROM unresolved
