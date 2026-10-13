-- veoveo: all tenants (the due pass wakes the server's parked Tasks of every tenant)
-- Queue up to $2 of server $1's Tasks parked on an operation whose next check is due,
-- earliest first. A Task another transaction holds waits for a later pass.
UPDATE tasks.task AS task
SET status = 'queued', status_message = 'Queued', parked_on = NULL
FROM (
    SELECT parked.id
    FROM tasks.task_operation AS o
    JOIN tasks.task AS parked
        ON parked.id = o.task_id AND parked.parked_on = o.operation_key
    WHERE o.server = $1 AND o.next_check_at <= now()
        AND o.state IN ('dispatching', 'submitted')
        AND parked.status = 'waiting'
    ORDER BY o.next_check_at
    LIMIT $2
    FOR UPDATE OF parked SKIP LOCKED
) AS due
WHERE task.id = due.id
