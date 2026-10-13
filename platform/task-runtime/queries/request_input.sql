-- veoveo: all tenants (a server's workers address the Tasks they run by ID, whichever tenant owns them)
-- The owning worker asks for input; the Task waits until a caller answers.
WITH parent AS (
    UPDATE tasks.task
    SET status = 'waiting', status_message = 'Waiting for input'
    WHERE id = $1 AND server = $2 AND owner_instance = $3 AND status IN ('running', 'waiting')
    RETURNING id
)
INSERT INTO tasks.task_input (task_id, request_key, method, params)
SELECT id, $4, $5, $6 FROM parent
RETURNING request_key, method, params, response, created_at, responded_at
