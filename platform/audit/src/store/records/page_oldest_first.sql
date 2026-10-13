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
    AND ($5::text IS NULL OR r.target = $5)
    AND ($6::text IS NULL OR r.outcome = $6)
    AND ($11::text IS NULL OR r.reason = $11)
    AND ($7::text IS NULL OR r.trace_id = $7)
    AND r.occurred_at >= coalesce($8::timestamptz, '-infinity')
    AND r.occurred_at < coalesce($9::timestamptz, 'infinity')
ORDER BY r.id
LIMIT $10
