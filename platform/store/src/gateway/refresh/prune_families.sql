-- Delete up to $2 families that expired at or before $1, earliest first, with their
-- tokens. A family that a rotation or revocation holds waits for a later batch.
DELETE FROM gateway.refresh_family
WHERE id IN (
    SELECT id
    FROM gateway.refresh_family
    WHERE expires_at <= $1
    ORDER BY expires_at
    LIMIT $2
    FOR UPDATE SKIP LOCKED
)
