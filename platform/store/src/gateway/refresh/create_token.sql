INSERT INTO gateway.refresh_token (
    id, family, token_digest, generation, issued_at, expires_at, delivery_envelope,
    delivery_expires_at
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
