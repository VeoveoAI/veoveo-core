-- A live row keeps its identifier; an expired one is claimed by exactly one caller.
INSERT INTO gateway.replay_id AS r
    (kind, authorization_server, client_id, jwt_id, seen_at, expires_at)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (kind, authorization_server, client_id, jwt_id) DO UPDATE
SET seen_at = excluded.seen_at, expires_at = excluded.expires_at
WHERE r.expires_at <= excluded.seen_at
RETURNING true AS "claimed!"
