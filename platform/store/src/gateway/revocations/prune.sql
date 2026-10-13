-- Delete up to $2 revocations of tokens that expired at or before $1, earliest first.
DELETE FROM gateway.jwt_revocation
WHERE (profile, issuer, jwt_id) IN (
    SELECT profile, issuer, jwt_id
    FROM gateway.jwt_revocation
    WHERE expires_at <= $1
    ORDER BY expires_at
    LIMIT $2
    FOR UPDATE SKIP LOCKED
)
