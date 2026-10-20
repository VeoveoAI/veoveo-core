-- veoveo: all tenants (a server's workers address the Tasks they run by ID, whichever tenant owns them)
-- A worker gives up its claim without an outcome. Clearing the owner fires the recovery
-- trigger, which moves the Task on as the end of its instance would.
UPDATE tasks.task
SET owner_instance = NULL
WHERE id = $1 AND server = $2 AND deployment = $4 AND owner_instance = $3
RETURNING
    id,
    server,
    deployment,
    tenant,
    owner_tenant,
    owner_principal,
    owner_kind AS "owner_kind: PrincipalKind",
    owner_issuer,
    owner_subject,
    surface,
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
