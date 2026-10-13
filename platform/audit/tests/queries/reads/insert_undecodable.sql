-- A record of another class whose draft holds only the members the lookup columns read.
-- It cannot decode, so a read that returns it fails.
INSERT INTO audit.record (draft, occurred_at)
SELECT
    jsonb_build_object(
        'id', $2::uuid,
        'partition', r.draft -> 'partition',
        'class', 'task.update',
        'outcome', r.draft -> 'outcome',
        'actor', r.draft -> 'actor',
        'authority', r.draft -> 'authority',
        'request', r.draft -> 'request',
        'target', r.draft -> 'target'
    ),
    r.occurred_at
FROM audit.record AS r
WHERE r.id = $1
