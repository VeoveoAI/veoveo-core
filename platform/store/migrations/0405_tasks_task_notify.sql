-- Every committed Task change wakes listeners of the `tasks` channel with the Task's ID
-- and tenant. Listeners reread the Task under their own authorization.
CREATE TRIGGER task_notify AFTER INSERT OR UPDATE OR DELETE ON tasks.task
FOR EACH ROW EXECUTE FUNCTION kernel.notify_change('id', 'tenant_id');
