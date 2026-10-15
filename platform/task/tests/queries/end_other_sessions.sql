-- End the other sessions of the calling login, as a network failure between a server and
-- the database does.
SELECT count(pg_catalog.pg_terminate_backend(pid)) AS "ended!"
FROM pg_catalog.pg_stat_activity
WHERE usename = session_user AND pid <> pg_catalog.pg_backend_pid()
