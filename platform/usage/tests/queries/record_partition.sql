-- The partition that holds the record of one source identifier.
SELECT tableoid::regclass::text AS "partition!" FROM usage.record WHERE source_id = $1
