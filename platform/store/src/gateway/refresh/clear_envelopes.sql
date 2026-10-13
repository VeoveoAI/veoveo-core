-- Clear up to $2 delivery envelopes whose window ended at or before $1, earliest window
-- first. Tokens another transaction holds wait for a later batch.
UPDATE gateway.refresh_token
SET delivery_envelope = NULL, delivery_expires_at = NULL
WHERE id IN (
    SELECT id
    FROM gateway.refresh_token
    WHERE delivery_expires_at <= $1
    ORDER BY delivery_expires_at
    LIMIT $2
    FOR UPDATE SKIP LOCKED
)
