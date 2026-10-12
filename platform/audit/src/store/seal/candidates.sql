-- Records after the sealed position whose transactions sort below the snapshot horizon,
-- in commit order. Every transaction below the horizon has ended, so no record can later
-- appear before the last one returned.
SELECT
    r.sequence,
    r.txid::text::bigint AS "txid!",
    r.id AS "id: AuditRecordId",
    r.partition,
    r.class AS "class: AuditClass",
    r.occurred_at,
    r.draft,
    r.recorded_at
FROM audit.sealer AS s
JOIN audit.record AS r ON (r.txid, r.sequence) > (s.txid, s.sequence)
WHERE r.txid < pg_snapshot_xmin(pg_current_snapshot())
ORDER BY r.txid, r.sequence
LIMIT $1
