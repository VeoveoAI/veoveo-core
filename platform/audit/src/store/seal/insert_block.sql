INSERT INTO audit.block (partition, sequence, head_hash, sealed_at, block)
VALUES ($1, $2, $3, $4, $5)
