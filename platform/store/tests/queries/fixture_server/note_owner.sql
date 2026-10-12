SELECT pg_catalog.pg_get_userbyid(c.relowner)::text AS owner
FROM pg_catalog.pg_class AS c
JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
WHERE n.nspname = 'fixture_server' AND c.relname = $1
