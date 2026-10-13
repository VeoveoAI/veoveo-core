-- Primary keys, foreign keys and check constraints outside the PostgreSQL schemas, with
-- their columns in key order.
SELECT
    k.conrelid::bigint AS relation,
    k.conname::text AS name,
    k.contype::text AS kind,
    ARRAY(
        SELECT a.attname::text
        FROM unnest(k.conkey) WITH ORDINALITY AS key (attnum, position)
        JOIN pg_catalog.pg_attribute AS a ON a.attrelid = k.conrelid AND a.attnum = key.attnum
        ORDER BY key.position
    ) AS columns,
    pg_catalog.pg_get_constraintdef(k.oid) AS definition
FROM pg_catalog.pg_constraint AS k
JOIN pg_catalog.pg_class AS c ON c.oid = k.conrelid
JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
WHERE k.contype IN ('p', 'f', 'c')
    AND n.nspname <> 'information_schema'
    AND n.nspname NOT LIKE 'pg\_%'
ORDER BY k.conrelid, k.conname
