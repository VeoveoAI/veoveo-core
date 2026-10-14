-- Whether a reaper marked the instance's lock as free, or None once the row is gone.
SELECT lock_free_since IS NOT NULL AS "marked!" FROM kernel.instance WHERE id = $1
