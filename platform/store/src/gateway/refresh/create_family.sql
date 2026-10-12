INSERT INTO gateway.refresh_family (
    id, authorization_server, profile, oauth_client_id, work_context, principal_id, tenant,
    scopes, principal, principal_display_name, current_generation, issued_at, expires_at
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 0, $11, $12)
