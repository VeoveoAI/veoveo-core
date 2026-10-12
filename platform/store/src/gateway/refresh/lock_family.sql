-- Every change to a family's tokens holds this lock, so presentations of one token
-- serialize across replicas.
SELECT
    f.id, f.authorization_server, f.profile, f.oauth_client_id, f.work_context,
    f.principal AS "principal: Json<Principal>", f.principal_display_name, f.scopes,
    f.current_generation, f.issued_at, f.expires_at, f.revoked_at,
    f.revocation_reason AS "revocation_reason: RefreshRevocation"
FROM gateway.refresh_family AS f
WHERE f.id = (SELECT t.family FROM gateway.refresh_token AS t WHERE t.token_digest = $1)
FOR UPDATE
