-- Whether any function of each name returns a set or aggregates rows.
SELECT
    n.nspname::text AS schema,
    p.proname::text AS name,
    bool_or(p.proretset) AS returns_set,
    bool_or(p.prokind = 'a') AS aggregate
FROM pg_catalog.pg_proc AS p
JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
GROUP BY n.nspname, p.proname
