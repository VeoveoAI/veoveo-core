-- Whether the instance's row is still there, which a reaper deletes once it finds the lock
-- free.
SELECT EXISTS (SELECT FROM kernel.instance WHERE id = $1) AS "present!"
