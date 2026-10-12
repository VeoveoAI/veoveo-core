INSERT INTO gateway.authorization_request (
    idp_state, profile, oauth_client_id, work_context, oidc_client, redirect_uri,
    client_state, requested_scopes, code_challenge, code_challenge_method,
    idp_code_verifier, idp_code_challenge, idp_code_challenge_method, nonce,
    created_at, expires_at
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
