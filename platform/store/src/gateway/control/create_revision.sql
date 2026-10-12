INSERT INTO gateway.control_revision
    (id, digest, source, applied_at, applied_by, tenant, control_plane)
VALUES ($1, $2, $3, $4, $5, $6, $7)
