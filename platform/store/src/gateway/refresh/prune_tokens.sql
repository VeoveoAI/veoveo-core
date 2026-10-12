DELETE FROM gateway.refresh_token
WHERE expires_at <= $1
