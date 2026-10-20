-- veoveo: all tenants (a server's workers address the Tasks they run by ID, whichever tenant owns them)
-- The owning worker records the provider's answer to the cancellation, once.
WITH parent AS (
    SELECT id
    FROM tasks.task
    WHERE id = $1 AND server = $2 AND deployment = $6 AND owner_instance = $3 AND status = 'cancel_requested'
    FOR SHARE
)
UPDATE tasks.task_operation AS o
SET cancel_receipt = $5, cancel_recorded_at = now()
FROM parent
WHERE o.task_id = parent.id AND o.operation_key = $4 AND o.cancel_receipt IS NULL
    AND o.cancel_started_at IS NOT NULL
RETURNING o.cancel_receipt AS "cancel_receipt!"
