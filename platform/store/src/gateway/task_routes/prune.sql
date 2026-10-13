-- Delete up to $2 Task routes that expired at or before $1, earliest first. A route a
-- concurrent projection holds waits for a later batch.
-- veoveo: all tenants (retention removes the expired routes of every tenant)
DELETE FROM gateway.task_route
WHERE id IN (
    SELECT id
    FROM gateway.task_route
    WHERE expires_at <= $1
    ORDER BY expires_at
    LIMIT $2
    FOR UPDATE SKIP LOCKED
)
