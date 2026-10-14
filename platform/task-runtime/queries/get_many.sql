-- veoveo: all tenants (a server reads the Tasks it holds by ID, whichever tenant owns them)
-- The Tasks of this server among the IDs in $2. A missing ID and another server's Task
-- both return no row.
SELECT
    id,
    server,
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
FROM tasks.task
WHERE server = $1 AND id = ANY ($2::uuid[])
ORDER BY id
