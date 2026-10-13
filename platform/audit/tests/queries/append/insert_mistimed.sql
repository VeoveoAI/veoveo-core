-- Store a copy of a record under a new identity with an occurrence time its draft does
-- not have.
INSERT INTO audit.record (draft, occurred_at)
SELECT jsonb_set(r.draft, '{id}', to_jsonb($2::uuid)), r.occurred_at + interval '1 second'
FROM audit.record AS r
WHERE r.id = $1
