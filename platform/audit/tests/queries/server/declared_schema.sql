-- The stored detail schema of a class.
SELECT t.schema AS "schema!"
FROM audit.detail_type AS t
WHERE t.class = $1
