-- veoveo: all tenants (a worker claims the oldest queued Task of its server, whichever tenant owns it)
UPDATE tasks.task AS task
SET status = 'running', status_message = 'Running', owner_instance = $2, started_at = now()
FROM (
    SELECT id
    FROM tasks.task
    WHERE server = $1 AND status = 'queued'
    ORDER BY created_at, id
    LIMIT 1
    FOR UPDATE SKIP LOCKED
) AS claimable
WHERE task.id = claimable.id
RETURNING
    task.id,
    task.server,
    task.tenant,
    task.owner_tenant,
    task.owner_principal,
    task.owner_kind AS "owner_kind: PrincipalKind",
    task.owner_issuer,
    task.owner_subject,
    task.profile,
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
