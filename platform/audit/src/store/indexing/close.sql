-- Remove an ended window whose read count is still the one the caller summarized.
DELETE FROM audit.indexing_window AS w
WHERE w.scope = $1
    AND w.start_at = $2
    AND w.reads = $3
    AND w.end_at <= now()
