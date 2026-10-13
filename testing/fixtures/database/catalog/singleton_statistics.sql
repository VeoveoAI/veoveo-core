-- Record that a singleton table holds one row on one page, so the planner stops costing
-- it as an unanalyzed table of ten pages.
SELECT pg_catalog.pg_restore_relation_stats(
    'schemaname', $1::text,
    'relname', $2::text,
    'relpages', 1::integer,
    'reltuples', 1::real
)
