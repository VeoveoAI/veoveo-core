SELECT
    control_plane_digest AS "control_plane_digest!",
    policy_revision AS "policy_revision!",
    context_digest AS "context_digest!",
    profile_policy_digest
FROM gateway.artifact_upload_authority($1, $2)
