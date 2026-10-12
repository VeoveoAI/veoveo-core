DELETE FROM gateway.jwt_revocation
WHERE expires_at <= $1
