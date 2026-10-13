-- The minimal audit record: an owner-defined reason code instead of the platform's
-- reason vocabulary, the target as a URI, the acting server, the outcomes success,
-- denied, failure and unknown, and declarations that name their OCSF class and carry
-- the detail schema as documentation. Records in the earlier format cannot be read as
-- the new one, so the migration refuses to run while any exist; an installation without
-- a release resets its database instead.
DO $$
BEGIN
    IF EXISTS (SELECT FROM audit.record) THEN
        RAISE EXCEPTION 'audit records predate the minimal audit record'
            USING ERRCODE = 'object_not_in_prerequisite_state',
                HINT = 'Reset the installation''s database, then start the Gateway again.';
    END IF;
END
$$;

-- Each class names the OCSF 1.9 event class and activity its records map to, such as API
-- Activity (6003) with Read (2). Its detail schema documents the detail for readers;
-- writers check none.
ALTER TABLE audit.detail_type
    ADD COLUMN ocsf_class_uid integer NOT NULL DEFAULT 6003 CHECK (ocsf_class_uid > 0),
    ADD COLUMN ocsf_activity_id smallint NOT NULL DEFAULT 99 CHECK (ocsf_activity_id >= 0),
    ALTER COLUMN schema DROP NOT NULL;

COMMENT ON TABLE audit.detail_type IS
    'Retention: for the life of the installation; a declaration changes in place.';

DROP VIEW audit.daily;
DROP INDEX audit.record_daily;

ALTER TABLE audit.record DROP COLUMN outcome;

ALTER TABLE audit.record
    ADD COLUMN outcome text GENERATED ALWAYS AS (draft ->> 'outcome') STORED NOT NULL
        CHECK (outcome IN ('success', 'denied', 'failure', 'unknown')),
    -- The record's source: the owner of its class, which only that owner writes.
    ADD COLUMN source text GENERATED ALWAYS AS (split_part(draft ->> 'class', '.', 1)) STORED NOT NULL,
    ADD COLUMN reason text GENERATED ALWAYS AS (draft ->> 'reason') STORED
        CHECK (reason IS NULL OR reason ~ '^[a-z][a-z0-9_]{0,63}$'),
    ADD COLUMN target text GENERATED ALWAYS AS (draft ->> 'target') STORED
        CHECK (target IS NULL OR octet_length(target) BETWEEN 1 AND 2048),
    ADD CONSTRAINT record_detail CHECK (jsonb_typeof(draft -> 'detail') = 'object');

-- Daily counts group one partition's records by UTC day, class and outcome.
CREATE INDEX record_daily ON audit.record (partition, day, class, outcome);

-- Filters by source and class over time, by outcome and reason, and by target each seek
-- an index that ends in the record identity or the occurrence time.
CREATE INDEX record_source ON audit.record (partition, source, class, occurred_at);
CREATE INDEX record_outcome ON audit.record (partition, outcome, reason, id);
CREATE INDEX record_target ON audit.record (partition, target, id) WHERE target IS NOT NULL;

-- Record counts per partition, UTC day, class and outcome over the retained records.
CREATE VIEW audit.daily AS
SELECT partition, day, class, outcome, count(*) AS count
FROM audit.record
GROUP BY partition, day, class, outcome;

REVOKE EXECUTE ON FUNCTION audit.declare_server_classes(text[], jsonb[]) FROM veoveo_server;
DROP FUNCTION audit.declare_server_classes(text[], jsonb[]);

-- Declares or replaces the calling server's classes. The calling login is session_user,
-- because this function runs as its owner. Every class must lie under the prefix of a
-- server whose group role the login holds, and is stored with that server; a class that
-- the installation owner or another server declared is refused with insufficient
-- privilege. A schema, when present, documents the detail as a JSON object of at most
-- 64 KiB. Repeating a declaration changes nothing, and a changed declaration replaces
-- the server's own one.
CREATE FUNCTION audit.declare_server_classes(
    classes text[], schemas jsonb[], ocsf_classes integer[], ocsf_activities smallint[]
)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    servers text[] := kernel.servers_of(session_user);
    refused text;
BEGIN
    IF cardinality(classes) > 256
        OR cardinality(classes) IS DISTINCT FROM cardinality(schemas)
        OR cardinality(classes) IS DISTINCT FROM cardinality(ocsf_classes)
        OR cardinality(classes) IS DISTINCT FROM cardinality(ocsf_activities)
    THEN
        RAISE EXCEPTION 'a server declares 0 to 256 classes, each with one schema and one OCSF class and activity'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    SELECT d.class INTO refused
    FROM unnest(classes) AS d (class)
    WHERE NOT split_part(d.class, '.', 1) = ANY (servers)
    LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'audit class % lies outside the calling server''s prefix', refused
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    SELECT d.class INTO refused
    FROM unnest(classes, schemas) AS d (class, schema)
    WHERE d.schema IS NOT NULL
        AND (jsonb_typeof(d.schema) <> 'object' OR octet_length(d.schema::text) > 65536)
    LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'the detail schema of audit class % is not a JSON object of at most 64 KiB',
            refused
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    INSERT INTO audit.detail_type AS t (class, schema, ocsf_class_uid, ocsf_activity_id, server)
    SELECT d.class, d.schema, d.ocsf_class, d.ocsf_activity, split_part(d.class, '.', 1)
    FROM unnest(classes, schemas, ocsf_classes, ocsf_activities)
        AS d (class, schema, ocsf_class, ocsf_activity)
    ON CONFLICT ON CONSTRAINT detail_type_pkey DO UPDATE SET
        schema = excluded.schema,
        ocsf_class_uid = excluded.ocsf_class_uid,
        ocsf_activity_id = excluded.ocsf_activity_id,
        declared_at = now()
    WHERE t.server = excluded.server
        AND (t.schema, t.ocsf_class_uid, t.ocsf_activity_id)
            IS DISTINCT FROM (excluded.schema, excluded.ocsf_class_uid, excluded.ocsf_activity_id);
    -- The insert left a class another owner declared untouched; refuse it.
    SELECT t.class INTO refused
    FROM audit.detail_type AS t
    WHERE t.class = ANY (classes) AND t.server IS DISTINCT FROM split_part(t.class, '.', 1)
    LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'audit class % belongs to another owner', refused
            USING ERRCODE = 'insufficient_privilege';
    END IF;
END
$$;

REVOKE EXECUTE ON FUNCTION
    audit.declare_server_classes(text[], jsonb[], integer[], smallint[])
FROM PUBLIC;

GRANT EXECUTE ON FUNCTION
    audit.declare_server_classes(text[], jsonb[], integer[], smallint[])
TO veoveo_server;
