-- Windows whose five minutes have ended by the database clock, oldest first.
SELECT
    w.scope,
    w.start_at,
    w.end_at,
    w.collection,
    w.template,
    w.reads,
    w.not_modified,
    w.failed,
    w.members_digest
FROM audit.indexing_window AS w
WHERE w.end_at <= now()
ORDER BY w.end_at
LIMIT 32
