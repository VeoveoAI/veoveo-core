-- Answer one outstanding input. The parent row lock orders the answer against
-- cancellation and completion, and an accepted answer advances the parent's revision.
WITH parent AS (
    SELECT id
    FROM tasks.task
    WHERE id = $1 AND server = $2 AND status IN ('queued', 'running', 'waiting')
    FOR UPDATE
),
answered AS (
    UPDATE tasks.task_input
    SET response = $4, responded_at = now()
    WHERE task_id = (SELECT id FROM parent) AND request_key = $3 AND response IS NULL
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
