-- Sessions of this database whose last statement subscribed to a channel.
SELECT pid AS "pid!"
FROM pg_stat_activity
WHERE datname = current_database()
    AND query LIKE 'LISTEN %'
    AND pid <> pg_backend_pid()
