-- The schemas that resolve unqualified names in this session, pg_catalog included.
SELECT pg_catalog.current_schemas(true)::text[] AS schemas
