-- The generic plan of a later page of daily counts with sequential scans disabled. The
-- prepared statement is src/store/records/daily.sql verbatim, which the test checks.
BEGIN;
SET LOCAL enable_seqscan = off;
PREPARE daily AS
-- One page of a partition's daily counts, newest day first, then by class and outcome.
-- After a cursor the days stop at the cursor's day, a bound that stays an index range in
-- a generic plan; the cursor's own day continues after its class and outcome.
SELECT
    d.day AS "day!",
    d.class AS "class!: AuditClass",
    d.outcome AS "outcome!: AuditOutcome",
    d.count AS "count!"
FROM audit.daily AS d
WHERE d.partition = $1
    AND d.day >= $2
    AND d.day < $3
    AND d.day <= coalesce($4::timestamptz, $3)
    AND (
        $4::timestamptz IS NULL
        OR d.day < $4
        OR (d.day = $4 AND (d.class, d.outcome) > ($5::text, $6::text))
    )
ORDER BY d.day DESC, d.class, d.outcome
LIMIT $7
;
SET LOCAL plan_cache_mode = force_generic_plan;
EXPLAIN (FORMAT JSON) EXECUTE daily(
    'installation', '2026-01-01T00:00:00Z', '2026-12-01T00:00:00Z', '2026-06-01T00:00:00Z',
    'api_activity', 'allowed', 101
);
DEALLOCATE daily;
ROLLBACK;
