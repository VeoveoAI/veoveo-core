SELECT
    id, family, token_digest, generation, issued_at, expires_at, consumed_at, replacement,
    replay_detected_at, delivery_envelope, delivery_expires_at
FROM gateway.refresh_token
WHERE token_digest = $1
