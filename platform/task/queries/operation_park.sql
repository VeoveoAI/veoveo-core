-- veoveo: all tenants (a server's workers address the Tasks they run by ID, whichever tenant owns them)
-- The owning worker parks its Task on a pending operation: the Task waits without an
-- owner until an event, a due check or the deadline queues it. The operation's row is
-- locked first, so a delivery that settles it either commits before and the Task is not
-- parked, or waits and then wakes the parked Task.
WITH pending AS (
    SELECT task_id
    FROM tasks.task_operation
    WHERE task_id = $1 AND operation_key = $4 AND server = $2
        AND state IN ('dispatching', 'submitted')
        AND (next_check_at IS NULL OR next_check_at > now())
    FOR UPDATE
)
UPDATE tasks.task AS task
SET status = 'waiting', status_message = $5, owner_instance = NULL, parked_on = $4
FROM pending
WHERE task.id = pending.task_id AND task.server = $2 AND task.owner_instance = $3
    AND task.status IN ('running', 'waiting')
RETURNING task.id
