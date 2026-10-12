INSERT INTO identity.work_context_membership
    (work_context, ordinal, level, principals, groups, roles, oauth_clients)
VALUES ($1, $2, $3, $4, $5, $6, $7)
