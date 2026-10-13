INSERT INTO gateway.registration (
    client_id, server, metadata, software_statement, access_token_digest, login,
    issued_at, updated_at
)
VALUES ($1, $2, $3, 'statement', $4, $5, now(), now())
