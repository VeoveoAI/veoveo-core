-- Every table, view and foreign table outside the PostgreSQL schemas, with its comment.
SELECT
    c.oid::bigint AS oid,
    n.nspname::text AS schema,
    c.relname::text AS name,
    c.relkind::text AS kind,
    c.relispartition AS partition,
    pg_catalog.obj_description(c.oid, 'pg_class') AS comment
FROM pg_catalog.pg_class AS c
JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f')
    AND n.nspname <> 'information_schema'
    AND n.nspname NOT LIKE 'pg\_%'
ORDER BY n.nspname, c.relname
