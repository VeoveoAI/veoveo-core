-- Delete up to $2 of server $1's finished Tasks whose retention expired and that no
-- consumer pins, earliest expiry first. A Task with an operation that has not settled
-- stays, so a late event still finds it. Their inputs, steps and operations cascade. A
-- Task another transaction holds waits for a later batch.
-- veoveo: all tenants (retention removes the server's expired Tasks of every tenant)
DELETE FROM tasks.task
WHERE id IN (
    SELECT id
    FROM tasks.task
    WHERE server = $1
        AND status IN ('succeeded', 'failed', 'cancelled')
        AND retention_pins = '{}'
        AND retention_expires_at <= now()
        AND NOT EXISTS (
            SELECT FROM tasks.task_operation AS o
            WHERE o.task_id = task.id AND o.state <> 'settled'
        )
    ORDER BY retention_expires_at
    LIMIT $2
    FOR UPDATE SKIP LOCKED
)
