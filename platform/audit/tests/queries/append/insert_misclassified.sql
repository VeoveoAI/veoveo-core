-- Store a copy of a record under a new identity with a class its draft does not have.
INSERT INTO audit.record (draft, class, occurred_at)
SELECT jsonb_set(r.draft, '{id}', to_jsonb($2::uuid)), 'authentication', r.occurred_at
FROM audit.record AS r
WHERE r.id = $1
