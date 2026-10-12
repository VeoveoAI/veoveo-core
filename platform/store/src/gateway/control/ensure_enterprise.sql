INSERT INTO identity.enterprise (id, slug, name)
VALUES ($1, 'installation', 'Veoveo installation')
ON CONFLICT (id) DO NOTHING
