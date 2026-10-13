-- Audit schema: records, signed blocks, partition heads, export delivery and retention
-- anchors. The installation owner reads and writes it. A registered server's login
-- declares its own classes and appends its own records through the two SECURITY
-- DEFINER functions at the end of this file, and holds no other privilege here.

CREATE SCHEMA audit;

ALTER DEFAULT PRIVILEGES IN SCHEMA audit REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC;

-- A partition's storage key: `installation`, or `tenant:` followed by the tenant.
CREATE FUNCTION audit.valid_partition(partition text) RETURNS boolean
LANGUAGE sql IMMUTABLE STRICT
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT valid_partition.partition = 'installation'
        OR (valid_partition.partition LIKE 'tenant:_%'
            AND octet_length(valid_partition.partition) <= 263
            AND valid_partition.partition !~ '[[:cntrl:]]')
$$;

-- A digest in `sha256:<lowercase hex>` form.
CREATE FUNCTION audit.valid_digest(digest text) RETURNS boolean
LANGUAGE sql IMMUTABLE STRICT
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT valid_digest.digest ~ '^sha256:[0-9a-f]{64}$'
$$;

-- An audit class and the JSON Schema of its detail, as the owner that records it
-- declared them. The class is `<owner>.<name>`. Writers check each detail against the
-- schema before they append it; a record's class must be declared here. `server` names
-- the registered server that declared the class through its login, which is always the
-- class's owner; it is NULL for a class the installation owner declared. It never
-- changes after the class is first declared.
CREATE TABLE audit.detail_type (
    class text PRIMARY KEY CHECK (class ~ '^[a-z][a-z0-9-]{0,47}\.[a-z][a-z0-9_]{0,62}$'),
    owner text GENERATED ALWAYS AS (split_part(class, '.', 1)) STORED NOT NULL,
    schema jsonb NOT NULL CHECK (jsonb_typeof(schema) = 'object'),
    declared_at timestamptz NOT NULL DEFAULT now(),
    server text CHECK (server IS NULL OR server = split_part(class, '.', 1))
);

-- One audit record. Producers supply the draft and its occurrence time; the lookup
-- columns are generated from the draft, so they cannot disagree with it. The database
-- assigns the insertion sequence, the writing transaction and the time of the insert.
CREATE TABLE audit.record (
    id uuid GENERATED ALWAYS AS ((draft ->> 'id')::uuid) STORED PRIMARY KEY,
    partition text GENERATED ALWAYS AS (
        CASE draft #>> '{partition,kind}'
            WHEN 'installation' THEN 'installation'
            WHEN 'tenant' THEN 'tenant:' || (draft #>> '{partition,tenant}')
        END
    ) STORED NOT NULL CHECK (audit.valid_partition(partition)),
    class text GENERATED ALWAYS AS (draft ->> 'class') STORED NOT NULL
        REFERENCES audit.detail_type (class),
    outcome text GENERATED ALWAYS AS (draft ->> 'outcome') STORED NOT NULL
        CHECK (outcome IN ('allowed', 'denied', 'succeeded', 'failed')),
    actor text GENERATED ALWAYS AS (draft #>> '{actor,principal}') STORED,
    trace_id text GENERATED ALWAYS AS (draft #>> '{request,trace_id}') STORED NOT NULL,
    occurred_at timestamptz NOT NULL,
    day timestamptz GENERATED ALWAYS AS (
        date_bin('1 day', occurred_at, timestamptz '1970-01-01 00:00:00+00')
    ) STORED NOT NULL,
    draft jsonb NOT NULL CHECK (jsonb_typeof(draft) = 'object'),
    sequence bigint GENERATED ALWAYS AS IDENTITY,
    txid xid8 NOT NULL DEFAULT pg_current_xact_id(),
    recorded_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

-- Pages read one partition in record identity order.
CREATE INDEX record_partition ON audit.record (partition, id);

-- The sealer reads records in commit order.
CREATE INDEX record_commit_order ON audit.record (txid, sequence);

-- Daily counts group one partition's records by UTC day, class and outcome.
CREATE INDEX record_daily ON audit.record (partition, day, class, outcome);

-- The foreign key to the record's class.
CREATE INDEX record_class ON audit.record (class);

-- The writing transaction and the database clock are assigned here even when an INSERT
-- names those columns, because the sealer's ordering depends on them.
CREATE FUNCTION audit.assign_record_commit() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    NEW.txid := pg_catalog.pg_current_xact_id();
    NEW.recorded_at := pg_catalog.clock_timestamp();
    RETURN NEW;
END
$$;

CREATE TRIGGER record_commit BEFORE INSERT ON audit.record
    FOR EACH ROW EXECUTE FUNCTION audit.assign_record_commit();

CREATE FUNCTION audit.reject_update() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    RAISE EXCEPTION 'audit.% rows are immutable', TG_TABLE_NAME
        USING ERRCODE = 'object_not_in_prerequisite_state';
END
$$;

CREATE TRIGGER record_immutable BEFORE UPDATE ON audit.record
    FOR EACH STATEMENT EXECUTE FUNCTION audit.reject_update();

-- Audit partitions are not identity tenants, so notifications carry no tenant. A
-- record's notification carries its identity.
CREATE TRIGGER record_notify AFTER INSERT ON audit.record
    FOR EACH ROW EXECUTE FUNCTION kernel.notify_change('id');

-- Record counts per partition, UTC day, class and outcome over the retained records.
CREATE VIEW audit.daily AS
SELECT partition, day, class, outcome, count(*) AS count
FROM audit.record
GROUP BY partition, day, class, outcome;

-- The commit position up to which every record is sealed. A record is sealed when its
-- (txid, sequence) is at most this pair.
CREATE TABLE audit.sealer (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    txid xid8 NOT NULL,
    sequence bigint NOT NULL CHECK (sequence >= 0)
);

INSERT INTO audit.sealer (txid, sequence) VALUES ('0', 0);

-- The signed head of each partition's newest block. Retention keeps it.
CREATE TABLE audit.partition_head (
    partition text PRIMARY KEY CHECK (audit.valid_partition(partition)),
    sequence bigint NOT NULL CHECK (sequence > 0),
    last_position bigint NOT NULL CHECK (last_position > 0),
    head_hash text NOT NULL CHECK (audit.valid_digest(head_hash))
);

-- A signed block: the canonical block document and the columns readers select by.
CREATE TABLE audit.block (
    partition text NOT NULL CHECK (audit.valid_partition(partition)),
    sequence bigint NOT NULL CHECK (sequence > 0),
    head_hash text NOT NULL CHECK (audit.valid_digest(head_hash)),
    sealed_at timestamptz NOT NULL,
    block jsonb NOT NULL CHECK (jsonb_typeof(block) = 'object'),
    PRIMARY KEY (partition, sequence)
);

CREATE TRIGGER block_immutable BEFORE UPDATE ON audit.block
    FOR EACH STATEMENT EXECUTE FUNCTION audit.reject_update();

-- A block's notification carries its partition, the part of its key that readers watch.
CREATE TRIGGER block_notify AFTER INSERT OR DELETE ON audit.block
    FOR EACH ROW EXECUTE FUNCTION kernel.notify_change('partition');

-- The last block retention removed from each partition. Verification of the retained
-- blocks starts from it.
CREATE TABLE audit.retention_anchor (
    partition text PRIMARY KEY CHECK (audit.valid_partition(partition)),
    sequence bigint NOT NULL CHECK (sequence > 0),
    block jsonb NOT NULL CHECK (jsonb_typeof(block) = 'object')
);

-- A delivery intent of one block to one destination, committed before the exporter
-- sends anything. Its hashes are immutable; a receipt sets delivered_at and a permanent
-- refusal sets rejection.
CREATE TABLE audit.export_delivery (
    partition text NOT NULL,
    sequence bigint NOT NULL,
    destination text NOT NULL CHECK (audit.valid_digest(destination)),
    content text NOT NULL CHECK (audit.valid_digest(content)),
    seal text NOT NULL CHECK (audit.valid_digest(seal)),
    created_at timestamptz NOT NULL DEFAULT now(),
    delivered_at timestamptz,
    rejection text CHECK (rejection IN (
        'partial_acceptance', 'protocol', 'content_conflict', 'object_lock',
        'payload_too_large'
    )),
    PRIMARY KEY (partition, sequence, destination),
    FOREIGN KEY (partition, sequence) REFERENCES audit.block ON DELETE CASCADE,
    CHECK (delivered_at IS NULL OR rejection IS NULL)
);

-- The newest block each destination acknowledged in each partition. Retention keeps it.
CREATE TABLE audit.export_cursor (
    destination text NOT NULL CHECK (audit.valid_digest(destination)),
    partition text NOT NULL CHECK (audit.valid_partition(partition)),
    sequence bigint NOT NULL CHECK (sequence > 0),
    head_hash text NOT NULL CHECK (audit.valid_digest(head_hash)),
    content text NOT NULL CHECK (audit.valid_digest(content)),
    seal text NOT NULL CHECK (audit.valid_digest(seal)),
    delivered_at timestamptz NOT NULL,
    PRIMARY KEY (destination, partition)
);

-- Records an export transition for the sealer, which holds the audit-sealer singleton.
-- `prepare` commits the intent, `complete` its receipt and `reject` a permanent refusal.
-- The block must be the next one the destination has not acknowledged in its partition.
CREATE FUNCTION audit.record_export(
    partition text,
    sequence bigint,
    head_hash text,
    destination text,
    content text,
    seal text,
    transition text,
    rejection text
) RETURNS void
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    block_hash text;
    intent audit.export_delivery;
    acknowledged bigint;
BEGIN
    IF record_export.transition NOT IN ('prepare', 'complete', 'reject')
        OR (record_export.transition = 'reject') <> (record_export.rejection IS NOT NULL)
    THEN
        RAISE EXCEPTION 'invalid audit export transition %', record_export.transition
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    SELECT b.head_hash INTO block_hash FROM audit.block AS b
    WHERE b.partition = record_export.partition AND b.sequence = record_export.sequence
    FOR SHARE;
    IF block_hash IS DISTINCT FROM record_export.head_hash THEN
        RAISE EXCEPTION 'audit block % of % does not match the export', record_export.sequence,
            record_export.partition USING ERRCODE = 'VV503';
    END IF;
    SELECT * INTO intent FROM audit.export_delivery AS d
    WHERE d.partition = record_export.partition
        AND d.sequence = record_export.sequence
        AND d.destination = record_export.destination
    FOR UPDATE;
    IF FOUND THEN
        IF intent.content <> record_export.content OR intent.seal <> record_export.seal THEN
            RAISE EXCEPTION 'audit export intent holds different payload hashes'
                USING ERRCODE = 'VV503';
        END IF;
        IF intent.rejection IS NOT NULL THEN
            RAISE EXCEPTION 'audit destination rejected this block' USING ERRCODE = 'VV504';
        END IF;
    END IF;
    IF NOT FOUND OR intent.delivered_at IS NULL THEN
        SELECT greatest(
            coalesce((SELECT c.sequence FROM audit.export_cursor AS c
                WHERE c.destination = record_export.destination
                    AND c.partition = record_export.partition FOR UPDATE), 0),
            coalesce((SELECT a.sequence FROM audit.retention_anchor AS a
                WHERE a.partition = record_export.partition FOR UPDATE), 0)
        ) INTO acknowledged;
        IF record_export.sequence <> acknowledged + 1 THEN
            RAISE EXCEPTION 'audit block % is not the next block for its destination',
                record_export.sequence USING ERRCODE = 'VV503';
        END IF;
    END IF;
    IF record_export.transition = 'prepare' THEN
        IF intent.partition IS NULL THEN
            INSERT INTO audit.export_delivery (partition, sequence, destination, content, seal)
            VALUES (record_export.partition, record_export.sequence,
                record_export.destination, record_export.content, record_export.seal);
        END IF;
    ELSIF intent.partition IS NULL THEN
        RAISE EXCEPTION 'audit export % has no committed intent', record_export.transition
            USING ERRCODE = 'VV503';
    ELSIF record_export.transition = 'reject' THEN
        IF intent.delivered_at IS NOT NULL THEN
            RAISE EXCEPTION 'a delivered audit export cannot be rejected' USING ERRCODE = 'VV503';
        END IF;
        UPDATE audit.export_delivery AS d SET rejection = record_export.rejection
        WHERE d.partition = record_export.partition
            AND d.sequence = record_export.sequence
            AND d.destination = record_export.destination;
    ELSIF intent.delivered_at IS NULL THEN
        UPDATE audit.export_delivery AS d SET delivered_at = now()
        WHERE d.partition = record_export.partition
            AND d.sequence = record_export.sequence
            AND d.destination = record_export.destination;
        INSERT INTO audit.export_cursor AS c
            (destination, partition, sequence, head_hash, content, seal, delivered_at)
        VALUES (record_export.destination, record_export.partition, record_export.sequence,
            record_export.head_hash, record_export.content, record_export.seal, now())
        ON CONFLICT ON CONSTRAINT export_cursor_pkey DO UPDATE SET
            sequence = excluded.sequence,
            head_hash = excluded.head_hash,
            content = excluded.content,
            seal = excluded.seal,
            delivered_at = excluded.delivered_at;
    END IF;
END
$$;

-- Removes the oldest retained block of a partition and its records once it was sealed
-- before the cutoff and every configured destination acknowledged it. The block becomes
-- the partition's retention anchor. Returns false when the block is not admitted.
CREATE FUNCTION audit.retire_block(
    partition text,
    sequence bigint,
    head_hash text,
    cutoff timestamptz,
    destinations text[],
    records uuid[]
) RETURNS boolean
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    retired audit.block;
    anchored bigint;
BEGIN
    SELECT * INTO retired FROM audit.block AS b
    WHERE b.partition = retire_block.partition AND b.sequence = retire_block.sequence
    FOR UPDATE;
    SELECT a.sequence INTO anchored FROM audit.retention_anchor AS a
    WHERE a.partition = retire_block.partition
    FOR UPDATE;
    IF NOT FOUND THEN
        anchored := 0;
    END IF;
    IF retired.partition IS NULL
        OR retired.head_hash <> retire_block.head_hash
        OR retired.sealed_at >= retire_block.cutoff
        OR retire_block.sequence <> anchored + 1
        OR EXISTS (
            SELECT FROM unnest(retire_block.destinations) AS required (destination)
            WHERE NOT EXISTS (
                SELECT FROM audit.export_delivery AS d
                WHERE d.partition = retire_block.partition
                    AND d.sequence = retire_block.sequence
                    AND d.destination = required.destination
                    AND d.delivered_at IS NOT NULL
            )
        )
    THEN
        RETURN false;
    END IF;
    DELETE FROM audit.record AS r
    WHERE r.partition = retire_block.partition AND r.id = ANY (retire_block.records);
    INSERT INTO audit.retention_anchor AS a (partition, sequence, block)
    VALUES (retire_block.partition, retire_block.sequence, retired.block)
    ON CONFLICT ON CONSTRAINT retention_anchor_pkey DO UPDATE SET
        sequence = excluded.sequence,
        block = excluded.block;
    DELETE FROM audit.block AS b
    WHERE b.partition = retire_block.partition AND b.sequence = retire_block.sequence;
    RETURN true;
END
$$;

-- Appends drafts inside the caller's transaction. A draft whose identity is already
-- stored must match the stored record exactly, so a retry changes nothing and a
-- conflicting reuse of an identity fails the transaction. A draft whose class is not
-- declared fails the foreign key to `audit.detail_type`.
CREATE FUNCTION audit.append(drafts jsonb[], occurred timestamptz[])
RETURNS void
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    matched bigint;
BEGIN
    IF cardinality(append.drafts) <> cardinality(append.occurred) THEN
        RAISE EXCEPTION 'audit append arrays differ in length'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    INSERT INTO audit.record (draft, occurred_at)
    SELECT i.draft, i.occurred_at
    FROM unnest(append.drafts, append.occurred) AS i (draft, occurred_at)
    ON CONFLICT ON CONSTRAINT record_pkey DO NOTHING;
    SELECT count(*) INTO matched
    FROM unnest(append.drafts, append.occurred) AS i (draft, occurred_at)
    JOIN audit.record AS r ON r.id = (i.draft ->> 'id')::uuid
        AND r.draft = i.draft
        AND r.occurred_at = i.occurred_at;
    IF matched <> cardinality(append.drafts) THEN
        RAISE EXCEPTION 'an audit record identity already holds different content'
            USING ERRCODE = 'VV501';
    END IF;
END
$$;

-- Server logins reach the audit schema only through the functions below.
GRANT USAGE ON SCHEMA audit TO veoveo_server;

-- Declares or replaces the calling server's classes. The calling login is session_user,
-- because this function runs as its owner. Every class must lie under the prefix of a
-- server whose group role the login holds, and is stored with that server; a class that
-- the installation owner or another server declared is refused with insufficient
-- privilege. Each schema must be a closed object of at most 64 KiB. Repeating a
-- declaration changes nothing, and a changed schema replaces the server's own one.
CREATE FUNCTION audit.declare_server_classes(classes text[], schemas jsonb[])
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    servers text[] := kernel.servers_of(session_user);
    refused text;
BEGIN
    IF cardinality(declare_server_classes.classes)
            IS DISTINCT FROM cardinality(declare_server_classes.schemas)
        OR cardinality(declare_server_classes.classes) > 256 THEN
        RAISE EXCEPTION 'a server declares 0 to 256 classes, each with one schema'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    SELECT d.class INTO refused
    FROM unnest(declare_server_classes.classes) AS d (class)
    WHERE NOT split_part(d.class, '.', 1) = ANY (servers)
    LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'audit class % lies outside the calling server''s prefix', refused
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    SELECT d.class INTO refused
    FROM unnest(declare_server_classes.classes, declare_server_classes.schemas)
        AS d (class, schema)
    WHERE d.schema IS NULL
        OR jsonb_typeof(d.schema) <> 'object'
        OR d.schema -> 'type' IS DISTINCT FROM '"object"'::jsonb
        OR d.schema -> 'additionalProperties' IS DISTINCT FROM 'false'::jsonb
        OR octet_length(d.schema::text) > 65536
    LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'the detail schema of audit class % is not a closed object of at most 64 KiB',
            refused
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    INSERT INTO audit.detail_type AS t (class, schema, server)
    SELECT d.class, d.schema, split_part(d.class, '.', 1)
    FROM unnest(declare_server_classes.classes, declare_server_classes.schemas)
        AS d (class, schema)
    ON CONFLICT ON CONSTRAINT detail_type_pkey DO UPDATE SET
        schema = excluded.schema,
        declared_at = now()
    WHERE t.server = excluded.server AND t.schema IS DISTINCT FROM excluded.schema;
    -- The insert left a class another owner declared untouched; refuse it.
    SELECT t.class INTO refused
    FROM audit.detail_type AS t
    WHERE t.class = ANY (declare_server_classes.classes)
        AND t.server IS DISTINCT FROM split_part(t.class, '.', 1)
    LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'audit class % belongs to another owner', refused
            USING ERRCODE = 'insufficient_privilege';
    END IF;
END
$$;

-- Appends the calling server's records inside the caller's transaction, through the
-- installation owner's `audit.append`, so they take the same commit ordering and
-- positions as every other record and commit or roll back with the caller's writes.
-- Each record's class must be declared by a server whose group role the calling login
-- holds; a class the installation owner or another server declared is refused with
-- insufficient privilege, and an undeclared class fails as `audit.append` fails it.
CREATE FUNCTION audit.append_server_records(drafts jsonb[], occurred timestamptz[])
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    servers text[] := kernel.servers_of(session_user);
    refused text;
BEGIN
    IF cardinality(append_server_records.drafts) > 4096 THEN
        RAISE EXCEPTION 'a server appends at most 4096 audit records at once'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    SELECT t.class INTO refused
    FROM unnest(append_server_records.drafts) AS i (draft)
    JOIN audit.detail_type AS t ON t.class = i.draft ->> 'class'
    WHERE t.server IS NULL OR NOT t.server = ANY (servers)
    LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'audit class % belongs to another owner', refused
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    PERFORM audit.append(append_server_records.drafts, append_server_records.occurred);
END
$$;

REVOKE EXECUTE ON FUNCTION
    audit.declare_server_classes(text[], jsonb[]),
    audit.append_server_records(jsonb[], timestamptz[])
FROM PUBLIC;

GRANT EXECUTE ON FUNCTION
    audit.declare_server_classes(text[], jsonb[]),
    audit.append_server_records(jsonb[], timestamptz[])
TO veoveo_server;
