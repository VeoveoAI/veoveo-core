-- End every other session of the calling login. A login may end its own sessions.
SELECT count(pg_catalog.pg_terminate_backend(pid))
FROM pg_catalog.pg_stat_activity
WHERE usename = session_user AND pid <> pg_catalog.pg_backend_pid()
