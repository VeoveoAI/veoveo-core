INSERT INTO gateway.namespace_claim (server, key_thumbprint, claimed_at)
VALUES ($1, $2, now())
