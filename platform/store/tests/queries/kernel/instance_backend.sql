-- The backend that holds an instance's lock.
SELECT pid AS "pid!"
FROM pg_locks
WHERE locktype = 'advisory'
    AND classid = 1987471726::oid
    AND objid = ($1::integer)::oid
    AND objsubid = 2
    AND granted
