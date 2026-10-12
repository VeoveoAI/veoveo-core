INSERT INTO tasks.task (
    id, server, tenant, owner_tenant, owner_principal, owner_kind, owner_issuer,
    owner_subject, profile, data_labels, work_context, authority, task_type, recovery_class,
    input, idempotency_key, ttl_ms, poll_interval_ms, retention_pins, status_message,
    retention_expires_at
)
VALUES (
    $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19,
    'Queued', now() + $20::bigint * interval '1 millisecond'
)
ON CONFLICT (server, tenant, owner_principal, profile, idempotency_key)
WHERE idempotency_key IS NOT NULL
DO NOTHING
RETURNING
    id AS "id: TaskId",
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
    owner_instance,
    revision,
    retention_pins,
    created_at,
    updated_at,
    started_at,
    cancel_requested_at,
    completed_at,
    retention_expires_at
