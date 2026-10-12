-- Move every open window into the previous five minutes. The finalizer still compares
-- against the database clock.
UPDATE audit.indexing_window
SET start_at = start_at - interval '5 minutes', end_at = end_at - interval '5 minutes'
