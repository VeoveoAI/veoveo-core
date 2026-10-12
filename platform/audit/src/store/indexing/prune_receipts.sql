-- Remove up to 1,024 receipts older than a day. Rows another replica is removing are
-- skipped rather than waited for.
WITH expired AS (
    SELECT i.partition, i.record
    FROM audit.indexing_receipt AS i
    WHERE i.recorded_at < now() - interval '1 day'
    ORDER BY i.recorded_at
    LIMIT 1024
    FOR UPDATE SKIP LOCKED
)
DELETE FROM audit.indexing_receipt AS i
USING expired
WHERE i.partition = expired.partition AND i.record = expired.record
