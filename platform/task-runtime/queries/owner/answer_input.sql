-- Answer one outstanding input of the caller's Task, selected by $1–$8 as in `get.sql`.
-- The parent row lock orders the answer against cancellation and completion, and an
-- accepted answer advances the parent's revision.
WITH parent AS (
    SELECT id
    FROM tasks.task
    WHERE server = $1
        AND tenant = $2
        AND owner_principal = $3
        AND profile = $4
        AND owner_tenant IS NOT DISTINCT FROM $5
        AND data_labels <@ $6
        AND ($7::text[] IS NULL OR task_type = ANY ($7))
        AND ($8::text IS NULL OR work_context = $8)
        AND id = $9
        AND status IN ('queued', 'running', 'waiting')
    FOR UPDATE
),
answered AS (
    UPDATE tasks.task_input
    SET response = $11, responded_at = now()
    WHERE task_id = (SELECT id FROM parent) AND request_key = $10 AND response IS NULL
    RETURNING task_id
),
touched AS (
    UPDATE tasks.task
    SET updated_at = now()
    WHERE id = (SELECT task_id FROM answered)
    RETURNING id
)
SELECT
    EXISTS (SELECT FROM parent) AS "admitted!",
    EXISTS (SELECT FROM touched) AS "accepted!"
