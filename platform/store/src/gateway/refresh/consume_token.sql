UPDATE gateway.refresh_token
SET consumed_at = $2, replacement = $3, delivery_envelope = NULL, delivery_expires_at = NULL
WHERE id = $1
