SELECT profile, issuer, jwt_id, revoked_at, expires_at, reason
FROM gateway.jwt_revocation
WHERE profile = $1 AND issuer = $2 AND jwt_id = $3 AND expires_at > $4
