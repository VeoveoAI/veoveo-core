-- A step of a Task's work. The owning worker records the step's operation before the
-- step runs and its result once it finishes, so a Task that runs again after recovery
-- skips the steps an earlier run finished and recognizes the ones it left unsettled.
CREATE TABLE tasks.task_step (
    task_id uuid NOT NULL REFERENCES tasks.task (id) ON DELETE CASCADE,
    -- The name the Task's work gives the step on every run.
    step_key text NOT NULL,
    -- Generated when the step is first dispatched and kept across runs, so a provider
    -- call can use it as an idempotency key.
    operation_id uuid NOT NULL,
    dispatched_at timestamptz NOT NULL DEFAULT now(),
    -- SQL NULL means no result yet; JSON null is a present result.
    result jsonb,
    completed_at timestamptz,
    PRIMARY KEY (task_id, step_key),
    CONSTRAINT task_step_key CHECK (
        octet_length(step_key) <= 128 AND tasks.valid_key(step_key)
    ),
    CONSTRAINT task_step_operation_v7 CHECK (
        uuid_extract_version(operation_id) = 7
        AND substr(operation_id::text, 20, 1) IN ('8', '9', 'a', 'b')
    ),
    CONSTRAINT task_step_completion CHECK ((result IS NULL) = (completed_at IS NULL))
);

-- A step keeps its Task, key, operation and dispatch time, and a finished step keeps
-- its result.
CREATE FUNCTION tasks.task_step_settle() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF (NEW.task_id, NEW.step_key, NEW.operation_id, NEW.dispatched_at)
        IS DISTINCT FROM (OLD.task_id, OLD.step_key, OLD.operation_id, OLD.dispatched_at)
        OR OLD.completed_at IS NOT NULL
    THEN
        RAISE EXCEPTION 'step % of task % keeps its operation and its result',
            OLD.step_key, OLD.task_id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER task_step_settle BEFORE UPDATE ON tasks.task_step
FOR EACH ROW EXECUTE FUNCTION tasks.task_step_settle();

ALTER TABLE tasks.task_step ENABLE ROW LEVEL SECURITY;

-- The parent read applies the Task policy, so a server sees only its own steps.
CREATE POLICY task_step_server ON tasks.task_step TO veoveo_server
USING (EXISTS (SELECT FROM tasks.task WHERE task.id = task_step.task_id))
WITH CHECK (EXISTS (SELECT FROM tasks.task WHERE task.id = task_step.task_id));

GRANT SELECT, INSERT, UPDATE, DELETE ON tasks.task_step TO veoveo_server;

COMMENT ON TABLE tasks.task_step IS
    'Retention: as long as its Task.';
