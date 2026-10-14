-- `page.sql` newest first. $1–$8 select the caller's Tasks as in `get.sql`. $9 and $10
-- are the creation time and ID of the previous page's last Task, and $11 limits the page.
-- The first page's bound follows every Task, so the keyset stays a backward index range
-- in a generic plan.
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
WHERE server = $1
    AND tenant = $2
    AND owner_principal = $3
    AND surface = $4
    AND owner_tenant IS NOT DISTINCT FROM $5
    AND data_labels <@ $6
    AND ($7::text[] IS NULL OR task_type = ANY ($7))
    AND ($8::text IS NULL OR work_context = $8)
    AND (created_at, id) < (
        coalesce($9::timestamptz, 'infinity'),
        coalesce($10::uuid, 'ffffffff-ffff-ffff-ffff-ffffffffffff')
    )
ORDER BY created_at DESC, id DESC
LIMIT $11
