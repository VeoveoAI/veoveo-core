-- Every committed Task change wakes listeners of its server's channel, `tasks.<server>`,
-- with the Task's ID and tenant, and no other channel. Listeners reread the Task under
-- their own authorization. The migration replaces the trigger in one transaction, so no
-- Task change commits between the two statements.
DROP TRIGGER task_notify ON tasks.task;

CREATE TRIGGER task_notify AFTER INSERT OR UPDATE OR DELETE ON tasks.task
FOR EACH ROW EXECUTE FUNCTION kernel.notify_change('id', 'tenant_id', 'server');
