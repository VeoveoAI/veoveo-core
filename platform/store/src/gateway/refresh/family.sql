SELECT
    id, authorization_server, profile, oauth_client_id, work_context,
    principal AS "principal: Json<Principal>", principal_display_name, scopes,
    current_generation, issued_at, expires_at, revoked_at,
    revocation_reason AS "revocation_reason: RefreshRevocation"
FROM gateway.refresh_family
WHERE id = $1
