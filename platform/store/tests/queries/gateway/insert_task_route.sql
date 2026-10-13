INSERT INTO gateway.task_route (
    id, tenant, owner, work_context, profile, server, source_task_id, authority_digest,
    owner_kind, owner_issuer, owner_subject, invocation_mode, initiator, delegation_id,
    data_labels, created_at, expires_at
)
VALUES (
    $1, $2, $3, $4, 'operator', 'upstream', $6, $5, 'user', 'https://issuer.example',
    'alice', 'automated', NULL, NULL, '{}', $7::timestamptz - interval '1 hour', $7
)
