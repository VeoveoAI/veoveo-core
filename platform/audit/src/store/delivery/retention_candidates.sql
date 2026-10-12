-- Each partition's oldest retained block, when it was sealed before the cutoff and every
-- configured destination acknowledged it.
SELECT b.partition, b.sequence, b.head_hash, b.sealed_at, b.block
FROM audit.partition_head AS h
LEFT JOIN audit.retention_anchor AS a ON a.partition = h.partition
JOIN audit.block AS b
    ON b.partition = h.partition AND b.sequence = coalesce(a.sequence, 0) + 1
WHERE b.sealed_at < $1
    AND NOT EXISTS (
        SELECT
        FROM unnest($2::text[]) AS required (destination)
        WHERE NOT EXISTS (
            SELECT
            FROM audit.export_delivery AS d
            WHERE d.partition = b.partition
                AND d.sequence = b.sequence
                AND d.destination = required.destination
                AND d.delivered_at IS NOT NULL
        )
    )
ORDER BY b.partition
LIMIT 64
