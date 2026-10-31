-- How many sessions of this database wait for a lock another transaction holds.
SELECT count(*) AS "waiting!"
FROM pg_stat_activity
WHERE datname = current_database() AND wait_event_type = 'Lock'
