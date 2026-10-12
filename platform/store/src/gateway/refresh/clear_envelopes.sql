UPDATE gateway.refresh_token
SET delivery_envelope = NULL, delivery_expires_at = NULL
WHERE delivery_expires_at <= $1
