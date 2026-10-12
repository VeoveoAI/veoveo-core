DELETE FROM gateway.authorization_request
WHERE idp_state = $1 AND expires_at > $2
RETURNING
    idp_state, profile, oauth_client_id, work_context, oidc_client, redirect_uri,
    client_state, requested_scopes, code_challenge, code_challenge_method,
    idp_code_verifier, idp_code_challenge, idp_code_challenge_method, nonce,
    created_at, expires_at
