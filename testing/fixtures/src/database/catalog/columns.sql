-- The columns of every relation that relations.sql reads, in column order.
SELECT
    a.attrelid::bigint AS relation,
    a.attname::text AS name,
    pg_catalog.format_type(a.atttypid, NULL) AS type_name,
    a.attgenerated::text <> '' AS generated
FROM pg_catalog.pg_attribute AS a
JOIN pg_catalog.pg_class AS c ON c.oid = a.attrelid
JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
WHERE a.attnum > 0
    AND NOT a.attisdropped
    AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
    AND n.nspname <> 'information_schema'
    AND n.nspname NOT LIKE 'pg\_%'
ORDER BY a.attrelid, a.attnum
