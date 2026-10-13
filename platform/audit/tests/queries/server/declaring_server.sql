-- The server that declared a class, or NULL when the installation owner declared it.
SELECT t.server
FROM audit.detail_type AS t
WHERE t.class = $1
