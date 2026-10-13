-- veoveo: all tenants (the sealer seals the records of every partition in commit order)
-- Records after the sealed position whose transactions sort below the snapshot horizon,
-- in commit order. Every transaction below the horizon has ended, so no record can later
-- appear before the last one returned. The position is read once, so the comparison
-- seeks record_commit_order past it instead of reading the sealed records before it.
SELECT
    r.sequence,
    r.txid::text::bigint AS "txid!",
    r.id AS "id: AuditRecordId",
    r.partition,
    r.class AS "class: AuditClass",
    r.occurred_at,
    r.draft,
    r.recorded_at
FROM audit.record AS r
WHERE (r.txid, r.sequence) > (SELECT s.txid, s.sequence FROM audit.sealer AS s)
    AND r.txid < pg_snapshot_xmin(pg_current_snapshot())
ORDER BY r.txid, r.sequence
LIMIT $1
