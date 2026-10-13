UPDATE gateway.registration
SET orphaned_at = CASE WHEN $2 THEN now() END
WHERE client_id = $1
RETURNING orphaned_at
