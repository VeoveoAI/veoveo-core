-- The principal rows of one tenant key and principal key.
SELECT count(*) AS "count!"
FROM identity.principal AS p
JOIN identity.tenant AS t ON t.id = p.tenant
WHERE t.slug = $1 AND p.principal_key = $2
