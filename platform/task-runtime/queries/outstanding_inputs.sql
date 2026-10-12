SELECT input.request_key, input.method, input.params, input.response, input.created_at,
    input.responded_at
FROM tasks.task_input AS input
JOIN tasks.task AS task ON task.id = input.task_id
WHERE task.id = $1 AND task.server = $2 AND input.response IS NULL
ORDER BY input.created_at, input.request_key
