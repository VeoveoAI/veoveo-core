-- The other sessions of the calling login.
SELECT pid
FROM pg_catalog.pg_stat_activity
WHERE usename = session_user AND pid <> pg_catalog.pg_backend_pid()
