-- The members of one sealed block that match a query, every filter applied in SQL. The
-- caller orders them by block position.
SELECT
    r.id AS "id: AuditRecordId",
    r.partition,
    r.class AS "class: AuditClass",
    r.occurred_at,
    r.draft,
    r.recorded_at
FROM audit.record AS r
WHERE r.partition = $1
    AND r.id = ANY ($2)
    AND ($3::text IS NULL OR r.class = $3)
    AND ($4::text IS NULL OR r.actor = $4)
    AND ($5::jsonb IS NULL OR r.draft -> 'target' = $5)
    AND ($6::text IS NULL OR r.outcome = $6)
    AND ($7::text IS NULL OR r.trace_id = $7)
    AND ($8::timestamptz IS NULL OR r.occurred_at >= $8)
    AND ($9::timestamptz IS NULL OR r.occurred_at < $9)
