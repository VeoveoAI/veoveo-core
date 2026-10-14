-- An instance whose lock connection failed takes its lock back on a new connection, so
-- its process keeps the work it owns when no reaper deleted the row in the meantime.
--
-- 'resumed': this session holds the lock again and the row is still there.
-- 'held': another session of the same login still holds the lock. Only the process's own
-- earlier connection takes it, and the process has left that connection, so the call
-- terminates that session and the caller tries again.
-- 'gone': the row was reaped, or another login started it, so the work it owned is in
-- recovery and the process needs a new instance.
CREATE FUNCTION kernel.resume_instance(instance integer) RETURNS text
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT FROM kernel.instance AS i
        WHERE i.id = resume_instance.instance
            AND i.login = session_user
    ) THEN
        RETURN 'gone';
    END IF;
    IF NOT pg_catalog.pg_try_advisory_lock(1987471726, resume_instance.instance) THEN
        PERFORM pg_catalog.pg_terminate_backend(l.pid)
        FROM pg_catalog.pg_locks AS l
        JOIN pg_catalog.pg_stat_activity AS a ON a.pid = l.pid
        WHERE l.locktype = 'advisory'
            AND l.database = (
                SELECT d.oid FROM pg_catalog.pg_database AS d
                WHERE d.datname = pg_catalog.current_database()
            )
            AND l.classid = 1987471726::oid
            AND l.objid = resume_instance.instance::oid
            AND l.objsubid = 2
            AND l.granted
            AND l.pid <> pg_catalog.pg_backend_pid()
            AND a.usename = session_user;
        RETURN 'held';
    END IF;
    -- A reaper may have deleted the row between the first check and the lock.
    IF NOT EXISTS (
        SELECT FROM kernel.instance AS i WHERE i.id = resume_instance.instance
    ) THEN
        PERFORM pg_catalog.pg_advisory_unlock(1987471726, resume_instance.instance);
        RETURN 'gone';
    END IF;
    RETURN 'resumed';
END
$$;

GRANT EXECUTE ON FUNCTION kernel.resume_instance(integer) TO veoveo_server;
