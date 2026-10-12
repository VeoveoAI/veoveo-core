-- Remove one member record and its block outside retention.
WITH removed AS (
    DELETE FROM audit.record AS r WHERE r.id = $1
)
DELETE FROM audit.block AS b
WHERE b.partition = $2 AND b.sequence = $3
