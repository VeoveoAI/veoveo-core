INSERT INTO identity.work_context
    (id, tenant, context_key, title, policy_revision, owner_kind, owner_key, classification, data_labels)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
