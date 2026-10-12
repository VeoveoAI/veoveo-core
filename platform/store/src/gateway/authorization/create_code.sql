INSERT INTO gateway.authorization_code (
    code, profile, oauth_client_id, work_context, oidc_client, redirect_uri,
    client_state, scopes, code_challenge, code_challenge_method, principal,
    principal_display_name, issued_at, expires_at, consumed_at
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
