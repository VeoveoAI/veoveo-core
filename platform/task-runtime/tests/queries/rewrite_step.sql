-- Change a recorded step directly, to probe the trigger that keeps its operation and
-- result.
UPDATE tasks.task_step
SET operation_id = coalesce($3, operation_id), result = coalesce($4, result),
    completed_at = CASE WHEN $4::jsonb IS NULL THEN completed_at ELSE now() END
WHERE task_id = $1 AND step_key = $2
