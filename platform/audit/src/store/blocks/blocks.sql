-- A partition's retained blocks after a sequence, in chain order.
SELECT b.partition, b.sequence, b.head_hash, b.sealed_at, b.block
FROM audit.block AS b
WHERE b.partition = $1 AND b.sequence > $2
ORDER BY b.sequence
LIMIT $3
