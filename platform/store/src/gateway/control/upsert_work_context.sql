INSERT INTO identity.work_context AS w (
    id, tenant, context_key, title, policy_revision, owner_kind, owner_key, classification,
    data_labels, created_at, updated_at
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10)
ON CONFLICT (id) DO UPDATE
SET title = excluded.title,
    policy_revision = excluded.policy_revision,
    owner_kind = excluded.owner_kind,
    owner_key = excluded.owner_key,
    classification = excluded.classification,
    data_labels = excluded.data_labels,
    updated_at = excluded.updated_at
