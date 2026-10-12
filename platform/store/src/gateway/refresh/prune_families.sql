DELETE FROM gateway.refresh_family
WHERE expires_at <= $1
