-- Record a step directly, as the installation owner, to probe the table's constraints.
INSERT INTO tasks.task_step (task_id, step_key, operation_id)
VALUES ($1, $2, $3)
