-- veoveo: all tenants (a worker claims the oldest queued Task of its server, whichever tenant owns it)
-- A claim takes a queued Task, which starts running, or a parked Task whose cancellation
-- the server must carry to its provider, which stays cancel-requested.
UPDATE tasks.task AS task
SET status = CASE WHEN task.status = 'queued' THEN 'running' ELSE task.status END,
    status_message = CASE WHEN task.status = 'queued' THEN 'Running' ELSE task.status_message END,
    owner_instance = $2,
    parked_on = NULL,
    started_at = CASE WHEN task.status = 'queued' THEN now() ELSE task.started_at END
FROM (
    SELECT id
    FROM tasks.task
    WHERE server = $1 AND deployment = $3
        AND (status = 'queued' OR (status = 'cancel_requested' AND owner_instance IS NULL))
    ORDER BY created_at, id
    LIMIT 1
    FOR UPDATE SKIP LOCKED
) AS claimable
WHERE task.id = claimable.id
RETURNING
    task.id,
    task.server,
    task.deployment,
    task.tenant,
    task.owner_tenant,
    task.owner_principal,
    task.owner_kind AS "owner_kind: PrincipalKind",
    task.owner_issuer,
    task.owner_subject,
    task.surface,
    task.data_labels,
    task.work_context,
    task.authority AS "authority: Json<InvocationAuthority>",
    task.task_type,
    task.recovery_class AS "recovery_class: RecoveryClass",
    task.input,
    task.idempotency_key,
    task.ttl_ms,
    task.poll_interval_ms,
    task.status AS "status: TaskStatus",
    task.status_message,
    task.progress,
    task.result,
    task.result_uri,
    task.error_code,
    task.error_message,
    task.error_details,
    task.owner_instance AS "owner_instance: InstanceId",
    task.revision,
    task.retention_pins,
    task.created_at,
    task.updated_at,
    task.started_at,
    task.cancel_requested_at,
    task.completed_at,
    task.retention_expires_at
