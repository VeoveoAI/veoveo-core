-- The tenant identifier derives from its key, so an existing row already holds it.
INSERT INTO identity.tenant (id, enterprise, slug, name)
VALUES ($1, $2, $3, $3)
ON CONFLICT (id) DO NOTHING
