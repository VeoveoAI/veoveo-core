-- How many of a block's member records the partition still holds.
SELECT count(*) AS "present!"
FROM audit.record AS r
WHERE r.partition = $1 AND r.id = ANY ($2)
