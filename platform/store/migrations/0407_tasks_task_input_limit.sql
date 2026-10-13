-- A Task holds at most 64 input requests. A request inserts its input in the statement
-- that locks the Task's row, so concurrent requests for one Task insert one after the
-- other, and each count reads the inputs committed before it.
CREATE FUNCTION tasks.task_input_limit() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF (SELECT count(*) FROM tasks.task_input WHERE task_id = NEW.task_id) >= 64 THEN
        RAISE EXCEPTION 'task % holds 64 input requests', NEW.task_id
            USING ERRCODE = 'check_violation', CONSTRAINT = 'task_input_limit';
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER task_input_limit BEFORE INSERT ON tasks.task_input
FOR EACH ROW EXECUTE FUNCTION tasks.task_input_limit();
