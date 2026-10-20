-- veoveo: all tenants (a server's workers address the Tasks they run by ID, whichever tenant owns them)
-- The owning worker records that it carries the Task's cancellation to the provider,
-- before it calls the provider.
WITH parent AS (
    SELECT id
    FROM tasks.task
    WHERE id = $1 AND server = $2 AND deployment = $5 AND owner_instance = $3 AND status = 'cancel_requested'
    FOR SHARE
)
UPDATE tasks.task_operation AS o
SET cancel_started_at = coalesce(o.cancel_started_at, now())
FROM parent
WHERE o.task_id = parent.id AND o.operation_key = $4 AND o.cancel_receipt IS NULL
RETURNING o.cancel_started_at AS "cancel_started_at!"
