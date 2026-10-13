-- Delete up to $3 identifiers of kind $1 that expired at or before $2, earliest first.
-- An identifier a concurrent claim holds waits for a later batch.
DELETE FROM gateway.replay_id
WHERE (kind, authorization_server, client_id, jwt_id) IN (
    SELECT kind, authorization_server, client_id, jwt_id
    FROM gateway.replay_id
    WHERE kind = $1 AND expires_at <= $2
    ORDER BY expires_at
    LIMIT $3
    FOR UPDATE SKIP LOCKED
)
