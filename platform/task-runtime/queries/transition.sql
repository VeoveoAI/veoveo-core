-- veoveo: all tenants (a server's workers address the Tasks they run by ID, whichever tenant owns them)
-- A worker transition: the caller's instance must own the Task at the expected revision.
-- A terminal status releases ownership.
UPDATE tasks.task
SET status = $5,
    status_message = $6,
    progress = $7,
    result = $8,
    result_uri = $9,
    error_code = $10,
    error_message = $11,
    error_details = $12,
    completed_at = CASE WHEN $5 IN ('succeeded', 'failed', 'cancelled') THEN now() END,
    owner_instance = CASE
        WHEN $5 IN ('succeeded', 'failed', 'cancelled') THEN NULL
        ELSE owner_instance
    END
WHERE id = $1 AND server = $2 AND owner_instance = $3 AND revision = $4
RETURNING
    id,
    server,
    tenant,
    owner_tenant,
    owner_principal,
    owner_kind AS "owner_kind: PrincipalKind",
    owner_issuer,
    owner_subject,
    profile,
    data_labels,
    work_context,
    authority AS "authority: Json<InvocationAuthority>",
    task_type,
    recovery_class AS "recovery_class: RecoveryClass",
    input,
    idempotency_key,
    ttl_ms,
    poll_interval_ms,
    status AS "status: TaskStatus",
    status_message,
    progress,
    result,
    result_uri,
    error_code,
    error_message,
    error_details,
    owner_instance AS "owner_instance: InstanceId",
    revision,
    retention_pins,
    created_at,
    updated_at,
    started_at,
    cancel_requested_at,
    completed_at,
    retention_expires_at
