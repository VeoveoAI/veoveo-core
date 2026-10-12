-- A Task loses its owner when the reaper deletes the owner's instance row. Unfinished
-- work then moves to the state its recovery class declares: a requested cancellation
-- completes, resumable work returns to the queue, and work whose effect may have
-- partly happened fails.
CREATE FUNCTION tasks.task_recovery() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF NEW.status = 'cancel_requested' THEN
        NEW.status := 'cancelled';
        NEW.status_message := 'Cancelled';
        NEW.completed_at := now();
    ELSIF NEW.recovery_class = 'resume' THEN
        NEW.status := 'queued';
        NEW.status_message := 'Queued';
        NEW.progress := 0;
        NEW.started_at := NULL;
    ELSE
        NEW.status := 'failed';
        NEW.error_code := 'interrupted_indeterminate';
        NEW.error_message := 'This task was interrupted and may have partly completed. '
            'It won''t be retried automatically. Check the result before running it again.';
        NEW.error_details := NULL;
        NEW.status_message := NEW.error_message;
        NEW.completed_at := now();
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER task_recovery BEFORE UPDATE OF owner_instance ON tasks.task
FOR EACH ROW
WHEN (
    OLD.owner_instance IS NOT NULL
    AND NEW.owner_instance IS NULL
    AND NEW.status IN ('running', 'waiting', 'cancel_requested')
)
EXECUTE FUNCTION tasks.task_recovery();
