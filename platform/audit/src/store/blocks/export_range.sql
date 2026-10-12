-- The partition's newest signed head and its last retired block, read in one snapshot.
SELECT
    h.sequence AS "sequence?",
    h.last_position AS "last_position?",
    h.head_hash AS "head_hash?",
    a.sequence AS "retired?"
FROM (SELECT $1::text AS partition) AS p
LEFT JOIN audit.partition_head AS h ON h.partition = p.partition
LEFT JOIN audit.retention_anchor AS a ON a.partition = p.partition
