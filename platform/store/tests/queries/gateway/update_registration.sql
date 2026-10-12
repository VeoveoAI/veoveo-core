UPDATE gateway.registration
SET metadata = $2, updated_at = now()
WHERE client_id = $1
