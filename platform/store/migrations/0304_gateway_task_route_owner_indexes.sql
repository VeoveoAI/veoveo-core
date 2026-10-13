-- Deleting a principal or a tenant checks the Task routes that reference it. These
-- indexes find those routes without a scan of every route.
CREATE INDEX task_route_owner ON gateway.task_route (owner);
CREATE INDEX task_route_tenant ON gateway.task_route (tenant);
