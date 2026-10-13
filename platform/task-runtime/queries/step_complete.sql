-- veoveo: all tenants (a server's workers address the Tasks they run by ID, whichever tenant owns them)
-- The owning worker records the result of the step it dispatched, once. The Task's row
-- stays locked until the statement commits.
WITH parent AS (
    SELECT id
    FROM tasks.task
    WHERE id = $1 AND server = $2 AND owner_instance = $3
        AND status IN ('running', 'waiting', 'cancel_requested')
    FOR SHARE
)
UPDATE tasks.task_step AS step
SET result = $6, completed_at = now()
FROM parent
WHERE step.task_id = parent.id AND step.step_key = $4 AND step.operation_id = $5
    AND step.completed_at IS NULL
RETURNING step.completed_at AS "completed_at!"
