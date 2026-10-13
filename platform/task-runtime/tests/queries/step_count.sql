-- The steps of one Task that the connected role can read.
SELECT count(*) AS "count!"
FROM tasks.task_step
WHERE task_id = $1
