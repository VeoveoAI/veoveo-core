-- veoveo: all tenants (usage kinds are installation-wide declarations, not tenant rows)
-- The installation owner declares or replaces usage kinds. Repeating a declaration
-- changes nothing.
INSERT INTO usage.kind AS t (kind, service_category, unit, monetary)
SELECT d.kind, d.category, d.unit, d.monetary
FROM unnest($1::text[], $2::text[], $3::text[], $4::boolean[]) AS d (kind, category, unit, monetary)
ON CONFLICT ON CONSTRAINT kind_pkey DO UPDATE SET
    service_category = excluded.service_category,
    unit = excluded.unit,
    monetary = excluded.monetary,
    declared_at = now()
WHERE t.server IS NULL
    AND (t.service_category, t.unit, t.monetary)
        IS DISTINCT FROM (excluded.service_category, excluded.unit, excluded.monetary)
