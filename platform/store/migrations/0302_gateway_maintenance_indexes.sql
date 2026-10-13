-- Deleting a refresh token clears the replacement that names it. This index finds that
-- predecessor without a scan of every token.
CREATE INDEX refresh_token_replacement ON gateway.refresh_token (replacement)
    WHERE replacement IS NOT NULL;

-- Cleanup deletes expired Task routes in expiry order, and removing a Work Context
-- removes its routes through the second index rather than a scan.
CREATE INDEX task_route_expiry ON gateway.task_route (expires_at);
CREATE INDEX task_route_work_context ON gateway.task_route (work_context);
