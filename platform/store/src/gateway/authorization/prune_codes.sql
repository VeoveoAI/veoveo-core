-- Delete up to $2 authorization codes that expired at or before $1, earliest first. A
-- code a redemption holds waits for a later batch.
DELETE FROM gateway.authorization_code
WHERE code IN (
    SELECT code
    FROM gateway.authorization_code
    WHERE expires_at <= $1
    ORDER BY expires_at
    LIMIT $2
    FOR UPDATE SKIP LOCKED
)
