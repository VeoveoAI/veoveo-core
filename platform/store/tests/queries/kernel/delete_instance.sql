-- Delete an instance's row as a reaper does once it finds the lock free.
DELETE FROM kernel.instance WHERE id = $1
