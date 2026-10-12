-- One page of a partition in record identity order, every filter applied before the limit.
SELECT
    r.id AS "id: AuditRecordId",
    r.partition,
    r.class AS "class: AuditClass",
    r.occurred_at,
    r.draft,
    r.recorded_at
FROM audit.record AS r
WHERE r.partition = $1
    AND ($2::uuid IS NULL OR r.id > $2)
    AND ($3::text IS NULL OR r.class = $3)
    AND ($4::text IS NULL OR r.actor = $4)
    AND ($5::jsonb IS NULL OR r.draft -> 'target' = $5)
    AND ($6::text IS NULL OR r.outcome = $6)
    AND ($7::text IS NULL OR r.trace_id = $7)
    AND ($8::timestamptz IS NULL OR r.occurred_at >= $8)
    AND ($9::timestamptz IS NULL OR r.occurred_at < $9)
ORDER BY r.id
LIMIT $10
