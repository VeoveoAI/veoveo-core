-- Plans of the oldest-first page with sequential scans disabled. The prepared statement
-- is src/store/records/page_oldest_first.sql verbatim, which the test checks: first a
-- generic plan of a later page, then custom plans filtered by actor and by trace.
BEGIN;
SET LOCAL enable_seqscan = off;
PREPARE page AS
-- One page of a partition in record identity order, every filter applied before the
-- limit. The cursor and time bounds stay index ranges in a generic plan: without a
-- cursor the bound is the nil UUID, which no record has, and an open time bound is
-- infinite.
SELECT
    r.id AS "id: AuditRecordId",
    r.partition,
    r.class AS "class: AuditClass",
    r.occurred_at,
    r.draft,
    r.recorded_at
FROM audit.record AS r
WHERE r.partition = $1
    AND r.id > coalesce($2::uuid, '00000000-0000-0000-0000-000000000000')
    AND ($3::text IS NULL OR r.class = $3)
    AND ($4::text IS NULL OR r.actor = $4)
    AND ($5::jsonb IS NULL OR r.draft -> 'target' = $5)
    AND ($6::text IS NULL OR r.outcome = $6)
    AND ($7::text IS NULL OR r.trace_id = $7)
    AND r.occurred_at >= coalesce($8::timestamptz, '-infinity')
    AND r.occurred_at < coalesce($9::timestamptz, 'infinity')
ORDER BY r.id
LIMIT $10
;
SET LOCAL plan_cache_mode = force_generic_plan;
EXPLAIN (FORMAT JSON) EXECUTE page(
    'installation', '0199d4a1-7c00-7b3e-9f2a-6c1d2e3f4a5b', NULL, NULL, NULL, NULL, NULL,
    NULL, NULL, 101
);
SET LOCAL plan_cache_mode = force_custom_plan;
EXPLAIN (FORMAT JSON) EXECUTE page(
    'installation', '0199d4a1-7c00-7b3e-9f2a-6c1d2e3f4a5b', NULL,
    'https://idp.example#alice', NULL, NULL, NULL, NULL, NULL, 101
);
EXPLAIN (FORMAT JSON) EXECUTE page(
    'installation', NULL, NULL, NULL, NULL, NULL, '4bf92f3577b34da6a3ce929d0e0e4736', NULL,
    NULL, 101
);
DEALLOCATE page;
ROLLBACK;
