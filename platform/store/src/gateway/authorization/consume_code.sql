UPDATE gateway.authorization_code
SET consumed_at = $2
WHERE code = $1 AND expires_at > $2 AND consumed_at IS NULL
RETURNING
    code, profile, oauth_client_id, work_context, oidc_client, redirect_uri,
    client_state, scopes, code_challenge, code_challenge_method,
    principal AS "principal: Json<Principal>", principal_display_name,
    issued_at, expires_at, consumed_at
