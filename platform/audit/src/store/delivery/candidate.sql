-- veoveo: all tenants (the exporter serves every partition, least recently served first)
-- The next block one destination has not acknowledged, from the partition it served
-- least recently. Partition heads, cursors and anchors select it; block history is not
-- scanned.
SELECT
    b.partition AS "partition?",
    b.sequence AS "sequence?",
    b.head_hash AS "head_hash?",
    b.sealed_at AS "sealed_at?",
    b.block AS "block?"
FROM audit.partition_head AS h
LEFT JOIN audit.export_cursor AS c ON c.destination = $1 AND c.partition = h.partition
LEFT JOIN audit.retention_anchor AS a ON a.partition = h.partition
LEFT JOIN audit.block AS b
    ON b.partition = h.partition
    AND b.sequence = greatest(coalesce(c.sequence, 0), coalesce(a.sequence, 0)) + 1
WHERE h.sequence > greatest(coalesce(c.sequence, 0), coalesce(a.sequence, 0))
ORDER BY c.delivered_at NULLS FIRST, h.partition
LIMIT 1
