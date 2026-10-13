-- veoveo: all tenants (a server's workers address the Tasks they run by ID, whichever tenant owns them)
-- The owning worker records a step's operation before the step runs, or reads the step an
-- earlier attempt recorded. The Task's row stays locked until the statement commits, so
-- the instance keeps the Task between the ownership check and the write.
WITH parent AS (
    SELECT id
    FROM tasks.task
    WHERE id = $1 AND server = $2 AND owner_instance = $3
        AND status IN ('running', 'waiting', 'cancel_requested')
    FOR SHARE
),
dispatched AS (
    INSERT INTO tasks.task_step (task_id, step_key, operation_id)
    SELECT id, $4, $5 FROM parent
    ON CONFLICT (task_id, step_key) DO NOTHING
    RETURNING operation_id, dispatched_at, result, completed_at
)
SELECT
    operation_id AS "operation_id!: StepOperationId",
    dispatched_at AS "dispatched_at!",
    result,
    completed_at,
    true AS "first!"
FROM dispatched
UNION ALL
SELECT step.operation_id, step.dispatched_at, step.result, step.completed_at, false
FROM tasks.task_step AS step
JOIN parent ON parent.id = step.task_id
WHERE step.step_key = $4
