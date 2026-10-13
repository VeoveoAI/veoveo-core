-- Delete up to $2 of server $1's finished Tasks whose retention expired and that no
-- consumer pins, earliest expiry first. Their input exchanges cascade. A Task another
-- transaction holds waits for a later batch.
-- veoveo: all tenants (retention removes the server's expired Tasks of every tenant)
DELETE FROM tasks.task
WHERE id IN (
    SELECT id
    FROM tasks.task
    WHERE server = $1
        AND status IN ('succeeded', 'failed', 'cancelled')
        AND retention_pins = '{}'
        AND retention_expires_at <= now()
    ORDER BY retention_expires_at
    LIMIT $2
    FOR UPDATE SKIP LOCKED
)
