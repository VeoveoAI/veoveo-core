-- Record an owner's class declarations. Repeating one changes nothing; a changed schema
-- replaces the stored one.
INSERT INTO audit.detail_type AS t (class, schema)
SELECT d.class, d.schema
FROM unnest($1::text[], $2::jsonb[]) AS d (class, schema)
ON CONFLICT ON CONSTRAINT detail_type_pkey DO UPDATE SET
    schema = excluded.schema,
    declared_at = now()
WHERE t.schema IS DISTINCT FROM excluded.schema
