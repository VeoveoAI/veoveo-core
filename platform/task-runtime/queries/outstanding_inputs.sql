-- veoveo: unbounded (a Task holds at most 64 input requests, which tasks.task_input_limit enforces)
-- veoveo: all tenants (a server's workers address the Tasks they run by ID, whichever tenant owns them)
SELECT input.request_key, input.kind, input.params, input.response, input.created_at,
    input.responded_at
FROM tasks.task_input AS input
JOIN tasks.task AS task ON task.id = input.task_id
WHERE task.id = $1 AND task.server = $2 AND input.response IS NULL
ORDER BY input.created_at, input.request_key
