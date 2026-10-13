-- veoveo: all tenants (a server's workers address the Tasks they run by ID, whichever tenant owns them)
SELECT input.request_key, input.method, input.params, input.response, input.created_at,
    input.responded_at
FROM tasks.task_input AS input
JOIN tasks.task AS task ON task.id = input.task_id
WHERE task.id = $1 AND task.server = $2 AND input.request_key = $3
