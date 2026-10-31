-- veoveo: all tenants (a server queues its Tasks of every tenant once the operations they are parked on stopped pending)
-- Queue server $1's Tasks parked on the listed operations, which the transaction just
-- settled or marked unresolved under their row locks. It runs as its own statement after
-- the one that changed them: that statement read the Tasks as they were when it began, so
-- it misses a park that held an operation's lock then and committed while it waited for
-- that lock. This statement begins after the lock was granted and sees that park.
UPDATE tasks.task AS task
SET status = 'queued', status_message = 'Queued', parked_on = NULL
FROM unnest($2::uuid[], $3::text[]) AS stopped (task_id, operation_key)
WHERE task.server = $1 AND task.id = stopped.task_id
    AND task.parked_on = stopped.operation_key AND task.status = 'waiting'
