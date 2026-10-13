-- The caller's own committed audit-view receipt for the partition, if it is current.
SELECT
    r.id AS "id: AuditRecordId",
    r.partition,
    r.class AS "class: AuditClass",
    r.occurred_at,
    r.draft,
    r.recorded_at
FROM audit.record AS r
WHERE r.id = $1
    AND r.partition = $2
    AND r.actor = $3
    AND r.draft -> 'detail' -> 'partition' = $4
    AND r.class = $5
    AND r.outcome = 'success'
    AND r.occurred_at > now() - interval '15 minutes'
