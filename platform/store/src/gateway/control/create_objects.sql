INSERT INTO gateway.control_object
    (revision, kind, object_id, tenant, document, profile_policy_version)
SELECT $1, o.kind, o.object_id, o.tenant, o.document, o.profile_policy_version
FROM unnest($2::text[], $3::text[], $4::text[], $5::jsonb[], $6::text[])
    AS o (kind, object_id, tenant, document, profile_policy_version)
