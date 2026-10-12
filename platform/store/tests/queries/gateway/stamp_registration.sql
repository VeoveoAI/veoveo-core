UPDATE gateway.registration
SET live_at = now()
WHERE client_id = $1
