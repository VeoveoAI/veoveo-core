-- Every declared class with its OCSF class and activity and its detail schema, for a
-- reader's registry.
-- veoveo: unbounded (one row per declared class; each owner declares a fixed vocabulary)
-- veoveo: scan (a reader's registry needs every declaration)
SELECT
    t.class AS "class: AuditClass",
    t.schema,
    t.ocsf_class_uid,
    t.ocsf_activity_id
FROM audit.detail_type AS t
ORDER BY t.class
