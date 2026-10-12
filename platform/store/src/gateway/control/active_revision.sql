SELECT
    r.id,
    r.digest,
    r.source,
    r.applied_at,
    r.applied_by,
    r.tenant,
    r.control_plane AS "control_plane: Json<GatewayControlPlane>"
FROM gateway.control_active AS a
JOIN gateway.control_revision AS r ON r.id = a.revision
