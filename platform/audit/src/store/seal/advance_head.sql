-- Make a block the partition's head. The first block creates the head; a later block
-- replaces it only when it directly follows the current head.
INSERT INTO audit.partition_head AS h (partition, sequence, last_position, head_hash)
VALUES ($1, $2, $3, $4)
ON CONFLICT (partition) DO UPDATE SET
    sequence = excluded.sequence,
    last_position = excluded.last_position,
    head_hash = excluded.head_hash
WHERE h.sequence + 1 = excluded.sequence AND h.head_hash = $5
