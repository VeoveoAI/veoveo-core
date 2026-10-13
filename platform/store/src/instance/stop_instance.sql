-- A clean shutdown deletes the instance's row and releases its lock before the connection
-- closes, so the instance reads as dead once shutdown returns rather than once the server
-- process behind the connection exits.
WITH stopped AS (
    DELETE FROM kernel.instance WHERE id = $1
)
SELECT pg_catalog.pg_advisory_unlock(1987471726, $1) AS "released!"
