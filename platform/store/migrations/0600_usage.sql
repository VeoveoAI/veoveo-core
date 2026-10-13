-- Usage schema: what servers metered for whom, in the columns of the FinOps Open Cost
-- and Usage Specification (FOCUS) 1.4. The installation owner reads and writes it. A
-- registered server's login declares its own kinds and appends its own records through
-- the SECURITY DEFINER functions below, and reads only its own rows.

CREATE SCHEMA usage;

ALTER DEFAULT PRIVILEGES IN SCHEMA usage REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC;

GRANT USAGE ON SCHEMA usage TO veoveo_server;

-- A kind of usage a server meters, such as `media.generation`: its FOCUS service
-- category, the FOCUS unit its quantities count, and whether its records carry an
-- amount of money. `server` names the registered server that declared the kind through
-- its login, which is always the kind's owner; it is NULL for a kind the installation
-- owner declared.
CREATE TABLE usage.kind (
    kind text PRIMARY KEY CHECK (kind ~ '^[a-z][a-z0-9-]{0,47}\.[a-z][a-z0-9_]{0,62}$'),
    owner text GENERATED ALWAYS AS (split_part(kind, '.', 1)) STORED NOT NULL,
    service_category text NOT NULL CHECK (service_category IN (
        'AI and Machine Learning', 'Analytics', 'Business Applications', 'Compute',
        'Databases', 'Developer Tools', 'Multicloud', 'Identity', 'Integration',
        'Internet of Things', 'Management and Governance', 'Media', 'Migration', 'Mobile',
        'Networking', 'Security', 'Storage', 'Web', 'Other'
    )),
    -- FOCUS Unit Format, such as `Tokens`, `Requests`, `Hours` or `GB-Hours`.
    unit text NOT NULL CHECK (unit ~ '^[A-Za-z0-9][A-Za-z0-9 /-]{0,63}$'),
    monetary boolean NOT NULL,
    declared_at timestamptz NOT NULL DEFAULT now(),
    server text CHECK (server IS NULL OR server = split_part(kind, '.', 1))
);

-- One usage record, partitioned by the month its charge period starts. The record's
-- source is the owner of its kind, so a server can only write its own usage.
CREATE TABLE usage.record (
    id uuid NOT NULL,
    tenant text NOT NULL,
    principal text NOT NULL,
    work_context text NOT NULL,
    kind text NOT NULL REFERENCES usage.kind (kind),
    source text GENERATED ALWAYS AS (split_part(kind, '.', 1)) STORED NOT NULL,
    acting_server text,
    acting_task uuid,
    task_id uuid,
    operation_key text,
    phase text NOT NULL CHECK (phase IN ('estimate', 'actual')),
    -- FOCUS ChargeCategory; ChargeClass `Correction` is the correction flag.
    charge_category text NOT NULL CHECK (charge_category IN ('Usage', 'Credit', 'Adjustment')),
    correction boolean NOT NULL,
    quantity numeric NOT NULL CHECK (quantity >= 0),
    amount numeric,
    -- ISO 4217 alphabetic code; `XXX` marks a record that involves no currency.
    currency text NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    target text CHECK (target IS NULL OR octet_length(target) BETWEEN 1 AND 2048),
    source_id text NOT NULL CHECK (
        octet_length(source_id) BETWEEN 1 AND 256 AND source_id !~ '[\u0001-\u001f\u007f-\u009f]'
    ),
    charge_period_start timestamptz NOT NULL,
    charge_period_end timestamptz NOT NULL,
    recorded_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (charge_period_start, id),
    CHECK (charge_period_start < charge_period_end),
    CHECK ((amount IS NULL) = (currency = 'XXX')),
    CHECK (acting_task IS NULL OR acting_server IS NOT NULL),
    CHECK (operation_key IS NULL OR task_id IS NOT NULL),
    CHECK (acting_server IS NULL OR acting_server ~ '^[a-z][a-z0-9-]{0,47}$')
) PARTITION BY RANGE (charge_period_start);

-- Records of months without a partition of their own.
CREATE TABLE usage.record_default PARTITION OF usage.record DEFAULT;

-- A tenant's records over time, a Task's records, and the foreign key to the kind.
CREATE INDEX record_tenant_period ON usage.record (tenant, charge_period_start, id);
CREATE INDEX record_task ON usage.record (task_id) WHERE task_id IS NOT NULL;
CREATE INDEX record_kind ON usage.record (kind);

-- The idempotency key of each record: its source, kind, phase and source identifier. A
-- partitioned table enforces uniqueness only within a partition, so the keys live here.
CREATE TABLE usage.record_key (
    source text NOT NULL,
    kind text NOT NULL,
    phase text NOT NULL,
    source_id text NOT NULL,
    record_id uuid NOT NULL,
    charge_period_start timestamptz NOT NULL,
    PRIMARY KEY (source, kind, phase, source_id)
);

-- Daily totals per tenant, principal, source, kind, phase and currency, maintained in
-- the transaction of each append.
CREATE TABLE usage.daily (
    tenant text NOT NULL,
    principal text NOT NULL,
    source text NOT NULL,
    kind text NOT NULL,
    day date NOT NULL,
    phase text NOT NULL,
    currency text NOT NULL,
    quantity numeric NOT NULL,
    amount numeric NOT NULL,
    records bigint NOT NULL,
    PRIMARY KEY (tenant, day, principal, source, kind, phase, currency)
);

-- Records and keys never change; corrections are new records.
CREATE FUNCTION usage.reject_update() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    RAISE EXCEPTION 'usage.% rows are immutable', TG_TABLE_NAME
        USING ERRCODE = 'object_not_in_prerequisite_state';
END
$$;

CREATE TRIGGER record_immutable BEFORE UPDATE ON usage.record
    FOR EACH STATEMENT EXECUTE FUNCTION usage.reject_update();

CREATE TRIGGER record_key_immutable BEFORE UPDATE ON usage.record_key
    FOR EACH STATEMENT EXECUTE FUNCTION usage.reject_update();

-- Appends usage drafts, each a JSON object with the record's columns, for the
-- installation owner. A repeated draft with the same source, kind, phase and source
-- identifier changes nothing; one that disagrees with the stored record raises VV601.
-- A record carries money exactly when its kind does. Each new record adds to its day's
-- totals in the same transaction.
CREATE FUNCTION usage.append(drafts jsonb[])
RETURNS bigint
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    d jsonb;
    stored usage.record;
    claimed uuid;
    money boolean;
    added bigint := 0;
BEGIN
    IF cardinality(append.drafts) > 4096 THEN
        RAISE EXCEPTION 'at most 4096 usage records at once'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    FOREACH d IN ARRAY coalesce(append.drafts, '{}') LOOP
        SELECT k.monetary INTO money FROM usage.kind AS k WHERE k.kind = d ->> 'kind';
        IF money IS DISTINCT FROM (d ->> 'currency' IS DISTINCT FROM 'XXX') THEN
            RAISE EXCEPTION 'usage of kind % must carry money exactly when its kind does',
                d ->> 'kind'
                USING ERRCODE = 'invalid_parameter_value';
        END IF;
        INSERT INTO usage.record_key (
            source, kind, phase, source_id, record_id, charge_period_start
        )
        VALUES (
            split_part(d ->> 'kind', '.', 1), d ->> 'kind', d ->> 'phase', d ->> 'source_id',
            (d ->> 'id')::uuid, (d ->> 'charge_period_start')::timestamptz
        )
        ON CONFLICT ON CONSTRAINT record_key_pkey DO NOTHING
        RETURNING record_id INTO claimed;
        IF claimed IS NULL THEN
            SELECT r.* INTO stored
            FROM usage.record_key AS k
            JOIN usage.record AS r
                ON r.charge_period_start = k.charge_period_start AND r.id = k.record_id
            WHERE k.source = split_part(d ->> 'kind', '.', 1) AND k.kind = d ->> 'kind'
                AND k.phase = d ->> 'phase' AND k.source_id = d ->> 'source_id';
            IF stored.quantity IS DISTINCT FROM (d ->> 'quantity')::numeric
                OR stored.amount IS DISTINCT FROM (d ->> 'amount')::numeric
                OR stored.currency IS DISTINCT FROM d ->> 'currency'
                OR stored.task_id IS DISTINCT FROM (d ->> 'task_id')::uuid
                OR stored.tenant IS DISTINCT FROM d ->> 'tenant'
            THEN
                RAISE EXCEPTION 'usage source % of kind % already holds a different record',
                    d ->> 'source_id', d ->> 'kind'
                    USING ERRCODE = 'VV601';
            END IF;
            CONTINUE;
        END IF;
        INSERT INTO usage.record (
            id, tenant, principal, work_context, kind, acting_server, acting_task, task_id,
            operation_key, phase, charge_category, correction, quantity, amount, currency,
            target, source_id, charge_period_start, charge_period_end
        )
        VALUES (
            (d ->> 'id')::uuid, d ->> 'tenant', d ->> 'principal', d ->> 'work_context',
            d ->> 'kind', d ->> 'acting_server', (d ->> 'acting_task')::uuid,
            (d ->> 'task_id')::uuid, d ->> 'operation_key', d ->> 'phase',
            d ->> 'charge_category', (d ->> 'correction')::boolean,
            (d ->> 'quantity')::numeric, (d ->> 'amount')::numeric, d ->> 'currency',
            d ->> 'target', d ->> 'source_id', (d ->> 'charge_period_start')::timestamptz,
            (d ->> 'charge_period_end')::timestamptz
        );
        INSERT INTO usage.daily AS t (
            tenant, principal, source, kind, day, phase, currency, quantity, amount, records
        )
        VALUES (
            d ->> 'tenant', d ->> 'principal', split_part(d ->> 'kind', '.', 1), d ->> 'kind',
            ((d ->> 'charge_period_start')::timestamptz AT TIME ZONE 'UTC')::date,
            d ->> 'phase', d ->> 'currency', (d ->> 'quantity')::numeric,
            coalesce((d ->> 'amount')::numeric, 0), 1
        )
        ON CONFLICT ON CONSTRAINT daily_pkey DO UPDATE SET
            quantity = t.quantity + excluded.quantity,
            amount = t.amount + excluded.amount,
            records = t.records + 1;
        added := added + 1;
    END LOOP;
    RETURN added;
END
$$;

-- Declares or replaces the calling server's usage kinds. The calling login is
-- session_user, because this function runs as its owner. Every kind must lie under the
-- prefix of a server whose group role the login holds; a kind the installation owner or
-- another server declared is refused with insufficient privilege.
CREATE FUNCTION usage.declare_server_kinds(
    kinds text[], categories text[], units text[], monetary boolean[]
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
    IF cardinality(kinds) > 256
        OR cardinality(kinds) IS DISTINCT FROM cardinality(categories)
        OR cardinality(kinds) IS DISTINCT FROM cardinality(units)
        OR cardinality(kinds) IS DISTINCT FROM cardinality(monetary)
    THEN
        RAISE EXCEPTION 'a server declares 0 to 256 usage kinds, each with one category, unit and money flag'
            USING ERRCODE = 'invalid_parameter_value';
    END IF;
    SELECT d.kind INTO refused
    FROM unnest(kinds) AS d (kind)
    WHERE NOT split_part(d.kind, '.', 1) = ANY (servers)
    LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'usage kind % lies outside the calling server''s prefix', refused
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    INSERT INTO usage.kind AS t (kind, service_category, unit, monetary, server)
    SELECT d.kind, d.category, d.unit, d.monetary, split_part(d.kind, '.', 1)
    FROM unnest(kinds, categories, units, monetary) AS d (kind, category, unit, monetary)
    ON CONFLICT ON CONSTRAINT kind_pkey DO UPDATE SET
        service_category = excluded.service_category,
        unit = excluded.unit,
        monetary = excluded.monetary,
        declared_at = now()
    WHERE t.server = excluded.server
        AND (t.service_category, t.unit, t.monetary)
            IS DISTINCT FROM (excluded.service_category, excluded.unit, excluded.monetary);
    SELECT t.kind INTO refused
    FROM usage.kind AS t
    WHERE t.kind = ANY (kinds) AND t.server IS DISTINCT FROM split_part(t.kind, '.', 1)
    LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'usage kind % belongs to another owner', refused
            USING ERRCODE = 'insufficient_privilege';
    END IF;
END
$$;

-- Appends the calling server's usage inside the caller's transaction through
-- `usage.append`. Each draft's kind must be declared by a server whose group role the
-- calling login holds.
CREATE FUNCTION usage.append_server_records(drafts jsonb[])
RETURNS bigint
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    servers text[] := kernel.servers_of(session_user);
    refused text;
BEGIN
    SELECT i.draft ->> 'kind' INTO refused
    FROM unnest(drafts) AS i (draft)
    LEFT JOIN usage.kind AS k ON k.kind = i.draft ->> 'kind'
    WHERE k.server IS NULL OR NOT k.server = ANY (servers)
    LIMIT 1;
    IF FOUND THEN
        RAISE EXCEPTION 'usage kind % is not declared by the calling server', refused
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    RETURN usage.append(drafts);
END
$$;

-- Creates the monthly partitions of `usage.record` from the current month through the
-- month of `through`, each named `record_<yyyy>_<mm>`. A month whose records already sit
-- in the default partition is skipped, because attaching it would move them.
CREATE FUNCTION usage.prepare_partitions(through date)
RETURNS integer
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    month date := date_trunc('month', now() AT TIME ZONE 'UTC')::date;
    created integer := 0;
    name text;
BEGIN
    WHILE month <= prepare_partitions.through LOOP
        name := format('record_%s', to_char(month, 'YYYY_MM'));
        IF to_regclass(format('usage.%I', name)) IS NULL AND NOT EXISTS (
            SELECT FROM usage.record_default
            WHERE charge_period_start >= month::timestamp AT TIME ZONE 'UTC'
                AND charge_period_start < (month + interval '1 month')::timestamp AT TIME ZONE 'UTC'
        ) THEN
            EXECUTE format(
                'CREATE TABLE usage.%I PARTITION OF usage.record FOR VALUES FROM (%L) TO (%L)',
                name,
                month::timestamp AT TIME ZONE 'UTC',
                (month + interval '1 month')::timestamp AT TIME ZONE 'UTC'
            );
            EXECUTE format(
                'COMMENT ON TABLE usage.%I IS %L',
                name,
                'Retention: the usage records of one month, until the installation drops the partition.'
            );
            created := created + 1;
        END IF;
        month := (month + interval '1 month')::date;
    END LOOP;
    RETURN created;
END
$$;

ALTER TABLE usage.record ENABLE ROW LEVEL SECURITY;
ALTER TABLE usage.daily ENABLE ROW LEVEL SECURITY;
ALTER TABLE usage.kind ENABLE ROW LEVEL SECURITY;

-- A server reads its own usage and kinds; it writes only through the functions above.
CREATE POLICY record_server ON usage.record FOR SELECT TO veoveo_server
USING (source = ANY ((SELECT kernel.current_servers())::text[]));

CREATE POLICY daily_server ON usage.daily FOR SELECT TO veoveo_server
USING (source = ANY ((SELECT kernel.current_servers())::text[]));

CREATE POLICY kind_server ON usage.kind FOR SELECT TO veoveo_server
USING (owner = ANY ((SELECT kernel.current_servers())::text[]));

GRANT SELECT ON usage.record, usage.daily, usage.kind TO veoveo_server;

REVOKE EXECUTE ON FUNCTION
    usage.declare_server_kinds(text[], text[], text[], boolean[]),
    usage.append_server_records(jsonb[])
FROM PUBLIC;

GRANT EXECUTE ON FUNCTION
    usage.declare_server_kinds(text[], text[], text[], boolean[]),
    usage.append_server_records(jsonb[])
TO veoveo_server;

COMMENT ON TABLE usage.kind IS
    'Retention: for the life of the installation; a declaration changes in place.';

COMMENT ON TABLE usage.record IS
    'Retention: by month; the installation drops a month''s partition when its retention '
    'passes.';

COMMENT ON TABLE usage.record_default IS
    'Retention: until the installation moves its records into monthly partitions.';

COMMENT ON TABLE usage.record_key IS
    'Retention: as long as the record it names.';

COMMENT ON TABLE usage.daily IS
    'Retention: as long as the records of its day.';
