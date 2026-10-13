-- Deleting a tenant or a principal checks the Tasks that reference it. These indexes
-- find those Tasks without a scan of every Task.
CREATE INDEX task_tenant_id ON tasks.task (tenant_id);
CREATE INDEX task_principal_id ON tasks.task (principal_id);
