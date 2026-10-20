-- Request cancellation of the caller's Task, selected by $1–$8 as in `get.sql`. An
-- unowned Task has no worker to observe the request, so it is cancelled at once, unless
-- it is parked on an operation: a claim then carries the cancellation to the provider.
UPDATE tasks.task
SET status = CASE
        WHEN owner_instance IS NULL AND parked_on IS NULL THEN 'cancelled'
        ELSE 'cancel_requested'
    END,
    status_message = CASE
        WHEN owner_instance IS NULL AND parked_on IS NULL THEN 'Cancelled'
        ELSE 'Cancelling'
    END,
    cancel_requested_at = now(),
    completed_at = CASE WHEN owner_instance IS NULL AND parked_on IS NULL THEN now() END
WHERE server = $1 AND deployment = $10
    AND tenant = $2
    AND owner_principal = $3
    AND surface = $4
    AND owner_tenant IS NOT DISTINCT FROM $5
    AND data_labels <@ $6
    AND ($7::text[] IS NULL OR task_type = ANY ($7))
    AND ($8::text IS NULL OR work_context = $8)
    AND id = $9
    AND status IN ('queued', 'running', 'waiting')
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
