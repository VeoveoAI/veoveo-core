-- The partition's newest signed head.
SELECT h.sequence, h.last_position, h.head_hash
FROM audit.partition_head AS h
WHERE h.partition = $1
