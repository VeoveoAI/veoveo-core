-- Every index outside the PostgreSQL schemas: its key columns in order, with NULL for an
-- expression, and its predicate when it is partial.
SELECT
    i.indrelid::bigint AS relation,
    ic.relname::text AS name,
    i.indisunique AS is_unique,
    ARRAY(
        SELECT a.attname::text
        FROM unnest(i.indkey::int2[]) WITH ORDINALITY AS k (attnum, position)
        LEFT JOIN pg_catalog.pg_attribute AS a
            ON a.attrelid = i.indrelid AND a.attnum = k.attnum AND k.attnum > 0
        WHERE k.position <= i.indnkeyatts
        ORDER BY k.position
    ) AS columns,
    pg_catalog.pg_get_expr(i.indpred, i.indrelid) AS predicate
FROM pg_catalog.pg_index AS i
JOIN pg_catalog.pg_class AS ic ON ic.oid = i.indexrelid
JOIN pg_catalog.pg_namespace AS n ON n.oid = ic.relnamespace
WHERE i.indisvalid
    AND n.nspname <> 'information_schema'
    AND n.nspname NOT LIKE 'pg\_%'
ORDER BY ic.relname
