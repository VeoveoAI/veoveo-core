-- Record an owner's class declarations. Repeating one changes nothing; a changed
-- declaration replaces the stored one.
INSERT INTO audit.detail_type AS t (class, schema, ocsf_class_uid, ocsf_activity_id)
SELECT d.class, d.schema, d.ocsf_class, d.ocsf_activity
FROM unnest($1::text[], $2::jsonb[], $3::integer[], $4::smallint[])
    AS d (class, schema, ocsf_class, ocsf_activity)
ON CONFLICT ON CONSTRAINT detail_type_pkey DO UPDATE SET
    schema = excluded.schema,
    ocsf_class_uid = excluded.ocsf_class_uid,
    ocsf_activity_id = excluded.ocsf_activity_id,
    declared_at = now()
WHERE (t.schema, t.ocsf_class_uid, t.ocsf_activity_id)
    IS DISTINCT FROM (excluded.schema, excluded.ocsf_class_uid, excluded.ocsf_activity_id)
