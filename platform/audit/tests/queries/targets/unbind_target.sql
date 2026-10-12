-- Store a record again with a target kind that no codec admits.
WITH removed AS (
    DELETE FROM audit.record AS r
    WHERE r.id = $1
    RETURNING r.draft, r.class, r.occurred_at
)
INSERT INTO audit.record (draft, class, occurred_at)
SELECT
    jsonb_set(removed.draft, '{target}', '{"kind": "missing_owner"}'),
    removed.class,
    removed.occurred_at
FROM removed
