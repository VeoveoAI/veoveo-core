UPDATE gateway.refresh_family
SET current_generation = $2
WHERE id = $1
