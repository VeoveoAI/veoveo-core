-- Delete up to $2 authorization requests that expired at or before $1, earliest first.
-- A request a callback is consuming waits for a later batch.
DELETE FROM gateway.authorization_request
WHERE idp_state IN (
    SELECT idp_state
    FROM gateway.authorization_request
    WHERE expires_at <= $1
    ORDER BY expires_at
    LIMIT $2
    FOR UPDATE SKIP LOCKED
)
