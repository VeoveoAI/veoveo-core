-- veoveo: all tenants (retention retires the oldest block of every partition)
-- Each partition's oldest retained block, when it was sealed before the cutoff and every
-- configured destination acknowledged it. Partition heads are read in key order; each
-- seeks its oldest block through the block key and counts that block's acknowledged
-- deliveries through the delivery key, so no other block or delivery is read.
SELECT b.partition, b.sequence, b.head_hash, b.sealed_at, b.block
FROM audit.partition_head AS h
LEFT JOIN audit.retention_anchor AS a ON a.partition = h.partition
CROSS JOIN LATERAL (
    SELECT o.partition, o.sequence, o.head_hash, o.sealed_at, o.block
    FROM audit.block AS o
    WHERE o.partition = h.partition
        AND o.sequence = coalesce(a.sequence, 0) + 1
        AND o.sealed_at < $1
        AND (
            SELECT count(*)
            FROM audit.export_delivery AS d
            WHERE d.partition = o.partition
                AND d.sequence = o.sequence
                AND d.destination = ANY ($2::text[])
                AND d.delivered_at IS NOT NULL
        ) = (SELECT count(DISTINCT required) FROM unnest($2::text[]) AS required)
    LIMIT 1
) AS b
ORDER BY h.partition
LIMIT 64
