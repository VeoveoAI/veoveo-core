-- Every committed Task change wakes the `tasks` channel. Listeners read current rows
-- under their own authorization; the payload only names the changed Task.
CREATE FUNCTION tasks.task_notify() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    changed tasks.task := CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
BEGIN
    PERFORM pg_notify(
        'tasks',
        json_build_object(
            'table', TG_TABLE_NAME,
            'op', lower(TG_OP),
            'id', changed.id,
            'server', changed.server,
            'tenant', changed.tenant
        )::text
    );
    RETURN NULL;
END;
$$;

CREATE TRIGGER task_notify AFTER INSERT OR UPDATE OR DELETE ON tasks.task
FOR EACH ROW EXECUTE FUNCTION tasks.task_notify();
