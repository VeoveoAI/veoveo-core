-- The partition's records among the given identities, in identity order.
SELECT
    r.id AS "id: AuditRecordId",
    r.partition,
    r.class AS "class: AuditClass",
    r.occurred_at,
    r.draft,
    r.recorded_at
FROM audit.record AS r
WHERE r.partition = $1 AND r.id = ANY ($2)
ORDER BY r.id
