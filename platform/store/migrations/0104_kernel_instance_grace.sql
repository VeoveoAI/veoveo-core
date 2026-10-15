-- A reaper deletes an instance only after its lock has stayed free for a grace period, so
-- a process whose lock connection dropped for a moment takes its lock back and keeps the
-- work it owns. The first reap that finds the lock free marks the row with the time; a
-- later reap deletes the row once that mark is older than the grace. An instance that
-- takes its lock back clears the mark, and so does any reap that finds the lock held.
ALTER TABLE kernel.instance ADD COLUMN lock_free_since timestamptz;

-- Marks the instances whose locks are free, deletes those whose locks have been free for
-- at least `grace`, and returns how many it deleted. A grace of zero deletes every
-- instance whose lock is free now. Work that references a deleted instance through
-- ON DELETE SET NULL loses its owner in the same transaction.
CREATE FUNCTION kernel.reap_instances(grace interval) RETURNS integer
LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    reaped integer;
BEGIN
    IF reap_instances.grace IS NULL OR reap_instances.grace < interval '0' THEN
        RAISE EXCEPTION 'an instance grace is a duration of zero or more'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    UPDATE kernel.instance AS i
    SET lock_free_since = NULL
    WHERE i.lock_free_since IS NOT NULL AND kernel.instance_alive(i.id);
    UPDATE kernel.instance AS i
    SET lock_free_since = now()
    WHERE i.lock_free_since IS NULL AND NOT kernel.instance_alive(i.id);
    WITH reaped_rows AS (
        DELETE FROM kernel.instance AS i
        WHERE i.lock_free_since <= now() - reap_instances.grace
            AND NOT kernel.instance_alive(i.id)
        RETURNING 1
    )
    SELECT count(*)::integer INTO reaped FROM reaped_rows;
    RETURN reaped;
END
$$;

-- The form a process on an earlier core calls during a rolling update. It applies the
-- default grace of `veoveo_store::INSTANCE_GRACE`, so such a process no longer
-- deletes an instance whose lock dropped a moment ago. It retires once every component
-- of an installation pins a core with `kernel.reap_instances(interval)`.
CREATE OR REPLACE FUNCTION kernel.reap_instances() RETURNS integer
LANGUAGE sql SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT kernel.reap_instances(interval '30 seconds')
$$;

-- Clears the free-lock mark of an instance whose lock the calling session holds.
CREATE FUNCTION kernel.instance_held(instance integer) RETURNS void
LANGUAGE sql SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    UPDATE kernel.instance AS i
    SET lock_free_since = NULL
    WHERE i.id = instance_held.instance
        AND i.lock_free_since IS NOT NULL
        AND EXISTS (
            SELECT FROM pg_catalog.pg_locks AS l
            WHERE l.locktype = 'advisory'
                AND l.database = (
                    SELECT d.oid FROM pg_catalog.pg_database AS d
                    WHERE d.datname = pg_catalog.current_database()
                )
                AND l.classid = 1987471726::oid
                AND l.objid = instance_held.instance::oid
                AND l.objsubid = 2
                AND l.granted
                AND l.pid = pg_catalog.pg_backend_pid()
        )
$$;

-- As in 0103, and an instance that takes its lock back clears its free-lock mark, so a
-- later drop of its connection starts a fresh grace.
CREATE OR REPLACE FUNCTION kernel.resume_instance(instance integer) RETURNS text
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
    PERFORM kernel.instance_held(resume_instance.instance);
    RETURN 'resumed';
END
$$;

GRANT EXECUTE ON FUNCTION
    kernel.reap_instances(interval),
    kernel.instance_held(integer)
TO veoveo_server;

COMMENT ON TABLE kernel.instance IS
    'Retention: while its process runs; a clean shutdown deletes the row, and '
    'kernel.reap_instances(grace) deletes it once its lock has been free for the grace.';
