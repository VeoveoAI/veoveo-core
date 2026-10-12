SELECT policy_revision,
  (SELECT VALUE revision.sha256 FROM ONLY gateway_control_active:current) AS control_plane_digest,
  crypto::sha256(<string> [policy_revision, memberships, output_policy]) AS context_digest,
  fn::artifacts::upload_profile_digest($profile_key) AS profile_policy_digest
FROM ONLY $context;
