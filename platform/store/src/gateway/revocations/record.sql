INSERT INTO gateway.jwt_revocation (profile, issuer, jwt_id, revoked_at, expires_at, reason)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (profile, issuer, jwt_id) DO UPDATE
SET revoked_at = excluded.revoked_at, expires_at = excluded.expires_at, reason = excluded.reason
