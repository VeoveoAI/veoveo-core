-- Delete up to $2 tokens that expired at or before $1, earliest first. Tokens another
-- transaction holds wait for a later batch.
DELETE FROM gateway.refresh_token
WHERE id IN (
    SELECT id
    FROM gateway.refresh_token
    WHERE expires_at <= $1
    ORDER BY expires_at
    LIMIT $2
    FOR UPDATE SKIP LOCKED
)
