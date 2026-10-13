-- The lookup the ON DELETE CASCADE action of task_route.work_context runs.
EXPLAIN (FORMAT JSON)
SELECT id FROM gateway.task_route WHERE work_context = $1
