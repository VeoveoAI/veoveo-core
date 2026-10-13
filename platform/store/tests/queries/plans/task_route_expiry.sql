-- The selection of one cleanup batch of expired Task routes.
EXPLAIN (FORMAT JSON)
SELECT id
FROM gateway.task_route
WHERE expires_at <= $1
ORDER BY expires_at
LIMIT $2
FOR UPDATE SKIP LOCKED
