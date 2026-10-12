-- Delete terminal Tasks whose retention expired and that no consumer pins. Their input
-- exchanges cascade.
DELETE FROM tasks.task
WHERE server = $1
    AND status IN ('succeeded', 'failed', 'cancelled')
    AND retention_pins = '{}'
    AND retention_expires_at <= now()
RETURNING id
