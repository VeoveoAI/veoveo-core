-- Audit schema: records, signed blocks, partition heads, export delivery, retention
-- anchors and indexing windows. Only the installation owner reads and writes it.

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

-- One audit record. Producers supply the draft, its class and its occurrence time; the
-- lookup columns are generated from the draft, so they cannot disagree with it. The
-- database assigns the insertion sequence, the writing transaction and the time of
-- the insert.
CREATE TABLE audit.record (
    id uuid GENERATED ALWAYS AS ((draft ->> 'id')::uuid) STORED PRIMARY KEY,
    partition text GENERATED ALWAYS AS (
        CASE draft #>> '{partition,kind}'
            WHEN 'installation' THEN 'installation'
            WHEN 'tenant' THEN 'tenant:' || (draft #>> '{partition,tenant}')
        END
    ) STORED NOT NULL CHECK (audit.valid_partition(partition)),
    class text NOT NULL CHECK (class IN (
        'api_activity', 'authentication', 'account_change', 'artifact_activity',
        'live_view_access', 'computer_activity'
    )),
    outcome text GENERATED ALWAYS AS (draft ->> 'outcome') STORED NOT NULL
        CHECK (outcome IN ('allowed', 'denied', 'succeeded', 'failed')),
    actor text GENERATED ALWAYS AS (draft #>> '{actor,principal}') STORED,
    profile text GENERATED ALWAYS AS (draft #>> '{authority,profile}') STORED,
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

-- An admitted read's retry receipt. A retry with the same fingerprint changes nothing;
-- one with another fingerprint fails.
CREATE TABLE audit.indexing_receipt (
    partition text NOT NULL CHECK (audit.valid_partition(partition)),
    record uuid NOT NULL,
    fingerprint text NOT NULL CHECK (audit.valid_digest(fingerprint)),
    recorded_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (partition, record)
);

CREATE INDEX indexing_receipt_expiry ON audit.indexing_receipt (recorded_at);

-- The open five-minute window of one actor, authority and collection. The finalizer
-- replaces it with one immutable record once the window has ended.
CREATE TABLE audit.indexing_window (
    scope text NOT NULL CHECK (audit.valid_digest(scope)),
    start_at timestamptz NOT NULL,
    end_at timestamptz NOT NULL,
    collection text NOT NULL,
    template jsonb NOT NULL CHECK (jsonb_typeof(template) = 'object'),
    reads bigint NOT NULL CHECK (reads > 0 AND reads <= 9007199254740991),
    not_modified bigint NOT NULL CHECK (not_modified >= 0),
    failed bigint NOT NULL CHECK (failed >= 0),
    members_digest text NOT NULL CHECK (audit.valid_digest(members_digest)),
    PRIMARY KEY (scope, start_at),
    CHECK (end_at = start_at + interval '5 minutes')
);

CREATE INDEX indexing_window_due ON audit.indexing_window (end_at);

-- Extends a window's member digest chain under
-- `https://veoveo.ai/formats/audit-indexing-members`. A read without a member keeps it.
CREATE FUNCTION audit.indexing_members(previous text, member text) RETURNS text
LANGUAGE sql IMMUTABLE
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT CASE
        WHEN indexing_members.member IS NULL THEN indexing_members.previous
        ELSE 'sha256:' || encode(sha256(convert_to(
            'https://veoveo.ai/formats/audit-indexing-members:'
                || substr(indexing_members.previous, 8)
                || substr(indexing_members.member, 8),
            'UTF8')), 'hex')
    END
$$;

-- Accumulates admitted indexing reads into the window of the current database time.
-- A new read must have occurred within the preceding hour and at most one minute ahead.
-- Reads apply in scope order, so concurrent groups lock windows in the same order.
CREATE FUNCTION audit.append_indexing(
    partitions text[],
    records uuid[],
    fingerprints text[],
    scopes text[],
    occurred timestamptz[],
    collections text[],
    templates jsonb[],
    members text[],
    not_modified bigint[],
    failed bigint[]
) RETURNS void
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    item record;
    window_start timestamptz := date_bin(
        '5 minutes', now(), timestamptz '1970-01-01 00:00:00+00'
    );
    genesis text := 'sha256:' || encode(sha256(convert_to(
        'https://veoveo.ai/formats/audit-indexing-members', 'UTF8')), 'hex');
    existing text;
BEGIN
    FOR item IN
        SELECT *
        FROM unnest(
            append_indexing.partitions, append_indexing.records,
            append_indexing.fingerprints, append_indexing.scopes,
            append_indexing.occurred, append_indexing.collections,
            append_indexing.templates, append_indexing.members,
            append_indexing.not_modified, append_indexing.failed
        ) AS r (partition_key, record_id, fingerprint, scope, occurred_at, collection,
            template, member, not_modified, failed)
        ORDER BY r.scope, r.partition_key, r.record_id
    LOOP
        INSERT INTO audit.indexing_receipt (partition, record, fingerprint)
        VALUES (item.partition_key, item.record_id, item.fingerprint)
        ON CONFLICT ON CONSTRAINT indexing_receipt_pkey DO NOTHING;
        IF NOT FOUND THEN
            SELECT i.fingerprint INTO existing FROM audit.indexing_receipt AS i
            WHERE i.partition = item.partition_key AND i.record = item.record_id;
            IF existing IS DISTINCT FROM item.fingerprint THEN
                RAISE EXCEPTION 'audit indexing read % was retried with different content',
                    item.record_id USING ERRCODE = 'VV501';
            END IF;
            CONTINUE;
        END IF;
        IF item.occurred_at < now() - interval '1 hour'
            OR item.occurred_at > now() + interval '1 minute'
        THEN
            RAISE EXCEPTION 'audit indexing read % is outside its admission window',
                item.record_id USING ERRCODE = 'VV502';
        END IF;
        INSERT INTO audit.indexing_window AS w (scope, start_at, end_at, collection, template,
            reads, not_modified, failed, members_digest)
        VALUES (item.scope, window_start, window_start + interval '5 minutes',
            item.collection, item.template, 1, item.not_modified, item.failed,
            audit.indexing_members(genesis, item.member))
        ON CONFLICT ON CONSTRAINT indexing_window_pkey DO UPDATE SET
            reads = w.reads + 1,
            not_modified = w.not_modified + excluded.not_modified,
            failed = w.failed + excluded.failed,
            members_digest = audit.indexing_members(w.members_digest, item.member);
    END LOOP;
END
$$;

-- Appends drafts inside the caller's transaction. A draft whose identity is already
-- stored must match the stored record exactly, so a retry changes nothing and a
-- conflicting reuse of an identity fails the transaction.
CREATE FUNCTION audit.append(drafts jsonb[], classes text[], occurred timestamptz[])
RETURNS void
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    matched bigint;
BEGIN
    IF cardinality(append.drafts) <> cardinality(append.classes)
        OR cardinality(append.drafts) <> cardinality(append.occurred)
    THEN
        RAISE EXCEPTION 'audit append arrays differ in length'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    INSERT INTO audit.record (draft, class, occurred_at)
    SELECT i.draft, i.class, i.occurred_at
    FROM unnest(append.drafts, append.classes, append.occurred)
        AS i (draft, class, occurred_at)
    ON CONFLICT ON CONSTRAINT record_pkey DO NOTHING;
    SELECT count(*) INTO matched
    FROM unnest(append.drafts, append.classes, append.occurred)
        AS i (draft, class, occurred_at)
    JOIN audit.record AS r ON r.id = (i.draft ->> 'id')::uuid
        AND r.draft = i.draft
        AND r.class = i.class
        AND r.occurred_at = i.occurred_at;
    IF matched <> cardinality(append.drafts) THEN
        RAISE EXCEPTION 'an audit record identity already holds different content'
            USING ERRCODE = 'VV501';
    END IF;
END
$$;
