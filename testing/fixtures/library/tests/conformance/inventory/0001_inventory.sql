-- A conforming owner schema. The installation owner applies it with the default search
-- path, so every name is qualified, and the migrator creates the schema and keeps its
-- history in inventory._sqlx_migrations.
CREATE TABLE inventory.item (
    id uuid PRIMARY KEY,
    tenant uuid NOT NULL,
    name text NOT NULL,
    UNIQUE (tenant, name)
);

COMMENT ON TABLE inventory.item IS 'Retention: until the tenant removes the item.';
