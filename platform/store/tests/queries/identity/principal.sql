SELECT
    tenant AS "tenant: TenantId",
    principal_key,
    kind AS "kind: PrincipalKind",
    issuer,
    subject,
    display_name
FROM identity.principal
WHERE id = $1
