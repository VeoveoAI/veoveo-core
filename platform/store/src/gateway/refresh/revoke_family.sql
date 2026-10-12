-- A family keeps the first revocation it received.
UPDATE gateway.refresh_family
SET revoked_at = coalesce(revoked_at, $2), revocation_reason = coalesce(revocation_reason, $3)
WHERE id = $1
RETURNING revoked_at AS "revoked_at!", revocation_reason AS "revocation_reason!: RefreshRevocation"
