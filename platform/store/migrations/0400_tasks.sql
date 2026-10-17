-- Tasks schema: durable Tasks, the inputs they request, the steps of their work and the
-- outside operations they wait on.

CREATE SCHEMA tasks;

ALTER DEFAULT PRIVILEGES IN SCHEMA tasks REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC;

GRANT USAGE ON SCHEMA tasks TO veoveo_server;

-- A Task key, surface, retention pin, input kind or idempotency key: 1 to 256 UTF-8 bytes
-- without a control character of Unicode category Cc.
CREATE FUNCTION tasks.valid_key(value text) RETURNS boolean
LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT octet_length(valid_key.value) BETWEEN 1 AND 256
        AND valid_key.value !~ '[\u0001-\u001f\u007f-\u009f]'
$$;

-- Retention pins are distinct valid keys.
CREATE FUNCTION tasks.valid_pins(pins text[]) RETURNS boolean
LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT coalesce(bool_and(tasks.valid_key(pin)), true)
        AND count(*) = count(DISTINCT pin)
    FROM unnest(valid_pins.pins) AS pin
$$;

-- Check constraints call these functions as the writing role.
GRANT EXECUTE ON FUNCTION tasks.valid_key(text), tasks.valid_pins(text[]) TO veoveo_server;

-- One durable Task. Columns that queries filter on are typed; the request input,
-- the result, failure details and the invocation authority snapshot are JSON.
CREATE TABLE tasks.task (
    id uuid PRIMARY KEY,
    server text NOT NULL,
    -- The owner's tenant key: its explicit claim, or `installation` without one.
    tenant text NOT NULL,
    tenant_id uuid NOT NULL REFERENCES identity.tenant (id),
    -- The owner's explicit tenant claim; absent for an installation-wide principal.
    owner_tenant text,
    owner_principal text NOT NULL,
    principal_id uuid NOT NULL REFERENCES identity.principal (id),
    owner_kind text NOT NULL,
    owner_issuer text NOT NULL,
    owner_subject text NOT NULL,
    -- The surface the owner submitted through, such as a Hub profile. The owner
    -- reads the Task only through the same surface.
    surface text NOT NULL,
    -- Data labels the owner held at submission; a reader must hold all of them.
    data_labels text[] NOT NULL DEFAULT '{}',
    work_context text NOT NULL,
    authority jsonb NOT NULL,
    task_type text NOT NULL,
    recovery_class text NOT NULL,
    input jsonb NOT NULL,
    idempotency_key text,
    ttl_ms bigint,
    poll_interval_ms bigint,
    status text NOT NULL DEFAULT 'queued',
    status_message text,
    progress double precision NOT NULL DEFAULT 0,
    -- SQL NULL means no result; JSON null is a present result.
    result jsonb,
    result_uri text,
    error_code text,
    error_message text,
    -- SQL NULL means no details; JSON null is present details.
    error_details jsonb,
    owner_instance integer REFERENCES kernel.instance (id) ON DELETE SET NULL,
    revision bigint NOT NULL DEFAULT 1,
    retention_pins text[] NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    started_at timestamptz,
    cancel_requested_at timestamptz,
    completed_at timestamptz,
    retention_expires_at timestamptz,
    -- The operation the Task is parked on. A parked Task waits without an owner, so the
    -- recovery trigger skips it and a restart leaves it waiting.
    parked_on text,

    CONSTRAINT task_id_v7 CHECK (
        uuid_extract_version(id) = 7 AND substr(id::text, 20, 1) IN ('8', '9', 'a', 'b')
    ),
    CONSTRAINT task_server CHECK (server ~ '^[a-z][a-z0-9-]{0,47}$'),
    CONSTRAINT task_owner_kind CHECK (owner_kind IN ('user', 'service')),
    CONSTRAINT task_surface CHECK (tasks.valid_key(surface)),
    CONSTRAINT task_owner_tenant CHECK (owner_tenant IS NULL OR owner_tenant = tenant),
    CONSTRAINT task_authority CHECK (
        jsonb_typeof(authority) = 'object'
        AND authority ->> 'tenant' = tenant
        AND authority ->> 'work_context' = work_context
    ),
    CONSTRAINT task_type_name CHECK (task_type ~ '^[a-z][a-z0-9._-]{0,127}$'),
    CONSTRAINT task_recovery_class CHECK (
        recovery_class IN ('resume', 'interrupted_indeterminate')
    ),
    CONSTRAINT task_status CHECK (
        status IN (
            'queued', 'running', 'waiting', 'succeeded', 'failed', 'cancel_requested',
            'cancelled'
        )
    ),
    CONSTRAINT task_idempotency_key CHECK (
        idempotency_key IS NULL OR tasks.valid_key(idempotency_key)
    ),
    CONSTRAINT task_ttl CHECK (ttl_ms >= 0),
    CONSTRAINT task_poll_interval CHECK (poll_interval_ms >= 0),
    CONSTRAINT task_progress CHECK (progress >= 0 AND progress <= 1),
    CONSTRAINT task_result CHECK ((result IS NOT NULL) = (status = 'succeeded')),
    CONSTRAINT task_result_uri CHECK (result_uri IS NULL OR status = 'succeeded'),
    CONSTRAINT task_error CHECK (
        (error_code IS NOT NULL) = (status = 'failed')
        AND (error_message IS NOT NULL) = (status = 'failed')
        AND (error_details IS NULL OR status = 'failed')
    ),
    CONSTRAINT task_completion CHECK (
        (completed_at IS NOT NULL) = (status IN ('succeeded', 'failed', 'cancelled'))
    ),
    CONSTRAINT task_owner_instance CHECK (
        owner_instance IS NULL OR status IN ('running', 'waiting', 'cancel_requested')
    ),
    CONSTRAINT task_cancel_requested CHECK (
        status <> 'cancel_requested' OR cancel_requested_at IS NOT NULL
    ),
    CONSTRAINT task_retention_pins CHECK (tasks.valid_pins(retention_pins)),
    CONSTRAINT task_revision CHECK (revision >= 1),
    CONSTRAINT task_parked CHECK (
        parked_on IS NULL
        OR (owner_instance IS NULL AND status IN ('waiting', 'cancel_requested'))
    )
);

COMMENT ON TABLE tasks.task IS
    'Retention: until retention_expires_at passes for a finished Task that no consumer '
    'pins; its server''s Task runtime prunes such Tasks in batches.';

-- A repeated creation with one key returns the existing Task; pruning releases the key.
CREATE UNIQUE INDEX task_idempotency ON tasks.task (
    server, tenant, owner_principal, surface, idempotency_key
) WHERE idempotency_key IS NOT NULL;

-- Owner reads, pages and subscriptions.
CREATE INDEX task_owner_page ON tasks.task (
    server, tenant, owner_principal, surface, created_at, id
);

-- Claims take queued Tasks and parked Tasks whose cancellation the server must carry to
-- the provider, oldest first.
CREATE INDEX task_claimable ON tasks.task (server, created_at, id)
WHERE status = 'queued' OR (status = 'cancel_requested' AND owner_instance IS NULL);

-- Reaping an instance clears ownership through this index.
CREATE INDEX task_owner_instance ON tasks.task (owner_instance)
WHERE owner_instance IS NOT NULL;

CREATE INDEX task_retention ON tasks.task (server, retention_expires_at)
WHERE status IN ('succeeded', 'failed', 'cancelled') AND retention_pins = '{}';

-- Deleting a tenant or a principal checks the Tasks that reference it. These indexes
-- find those Tasks without a scan of every Task.
CREATE INDEX task_tenant_id ON tasks.task (tenant_id);
CREATE INDEX task_principal_id ON tasks.task (principal_id);

ALTER TABLE tasks.task ENABLE ROW LEVEL SECURITY;

-- A registered server reads and writes the Tasks of its own servers. The scalar
-- subquery runs once per statement.
CREATE POLICY task_server ON tasks.task TO veoveo_server
USING (server = ANY ((SELECT kernel.current_servers())::text[]))
WITH CHECK (server = ANY ((SELECT kernel.current_servers())::text[]));

GRANT SELECT, INSERT, UPDATE, DELETE ON tasks.task TO veoveo_server;

-- Every update advances the revision and the update time. A Task's identity, owner and
-- request stay as it was created.
CREATE FUNCTION tasks.task_revision() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF (NEW.id, NEW.server, NEW.tenant, NEW.tenant_id, NEW.owner_tenant,
        NEW.owner_principal, NEW.principal_id, NEW.owner_kind, NEW.owner_issuer,
        NEW.owner_subject, NEW.surface, NEW.data_labels, NEW.work_context, NEW.authority,
        NEW.task_type, NEW.recovery_class, NEW.input, NEW.idempotency_key, NEW.ttl_ms,
        NEW.poll_interval_ms, NEW.created_at, NEW.retention_expires_at)
        IS DISTINCT FROM
       (OLD.id, OLD.server, OLD.tenant, OLD.tenant_id, OLD.owner_tenant,
        OLD.owner_principal, OLD.principal_id, OLD.owner_kind, OLD.owner_issuer,
        OLD.owner_subject, OLD.surface, OLD.data_labels, OLD.work_context, OLD.authority,
        OLD.task_type, OLD.recovery_class, OLD.input, OLD.idempotency_key, OLD.ttl_ms,
        OLD.poll_interval_ms, OLD.created_at, OLD.retention_expires_at)
    THEN
        RAISE EXCEPTION 'task % keeps its identity, owner and request', OLD.id
            USING ERRCODE = 'check_violation';
    END IF;
    NEW.revision := OLD.revision + 1;
    NEW.updated_at := now();
    RETURN NEW;
END
$$;

CREATE TRIGGER task_revision BEFORE UPDATE ON tasks.task
FOR EACH ROW EXECUTE FUNCTION tasks.task_revision();

-- A Task loses its owner when the reaper deletes the owner's instance row. Unfinished
-- work then moves to the state its recovery class declares: a requested cancellation
-- completes, resumable work returns to the queue, and work whose effect may have
-- partly happened fails. Recovery never applies to a Task its worker parked on an
-- operation.
CREATE FUNCTION tasks.task_recovery() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF NEW.status = 'cancel_requested' THEN
        NEW.status := 'cancelled';
        NEW.status_message := 'Cancelled';
        NEW.completed_at := now();
    ELSIF NEW.recovery_class = 'resume' THEN
        NEW.status := 'queued';
        NEW.status_message := 'Queued';
        NEW.progress := 0;
        NEW.started_at := NULL;
    ELSE
        NEW.status := 'failed';
        NEW.error_code := 'interrupted_indeterminate';
        NEW.error_message := 'This task was interrupted and may have partly completed. '
            'It won''t be retried automatically. Check the result before running it again.';
        NEW.error_details := NULL;
        NEW.status_message := NEW.error_message;
        NEW.completed_at := now();
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER task_recovery BEFORE UPDATE OF owner_instance ON tasks.task
FOR EACH ROW
WHEN (
    OLD.owner_instance IS NOT NULL
    AND NEW.owner_instance IS NULL
    AND NEW.status IN ('running', 'waiting', 'cancel_requested')
    AND NEW.parked_on IS NULL
)
EXECUTE FUNCTION tasks.task_recovery();

-- Every committed Task change wakes listeners of its server's channel, `tasks.<server>`,
-- with the Task's ID and tenant, and no other channel. Listeners reread the Task under
-- their own authorization.
CREATE TRIGGER task_notify AFTER INSERT OR UPDATE OR DELETE ON tasks.task
FOR EACH ROW EXECUTE FUNCTION kernel.notify_change('id', 'tenant_id', 'server');

-- An input a Task requested from its caller, and the caller's answer.
CREATE TABLE tasks.task_input (
    task_id uuid NOT NULL REFERENCES tasks.task (id) ON DELETE CASCADE,
    request_key text NOT NULL,
    -- What the Task asks for, named by the protocol adapter that relays the request.
    kind text NOT NULL,
    params jsonb NOT NULL,
    response jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    responded_at timestamptz,
    PRIMARY KEY (task_id, request_key),
    CONSTRAINT task_input_key CHECK (tasks.valid_key(request_key)),
    CONSTRAINT task_input_kind CHECK (tasks.valid_key(kind)),
    CONSTRAINT task_input_params CHECK (jsonb_typeof(params) = 'object'),
    CONSTRAINT task_input_response CHECK (
        response IS NULL OR jsonb_typeof(response) = 'object'
    ),
    CONSTRAINT task_input_answered CHECK ((response IS NULL) = (responded_at IS NULL))
);

COMMENT ON TABLE tasks.task_input IS
    'Retention: as long as its Task.';

ALTER TABLE tasks.task_input ENABLE ROW LEVEL SECURITY;

-- The parent read applies the Task policy, so a server sees only its own inputs.
CREATE POLICY task_input_server ON tasks.task_input TO veoveo_server
USING (EXISTS (SELECT FROM tasks.task WHERE task.id = task_input.task_id))
WITH CHECK (EXISTS (SELECT FROM tasks.task WHERE task.id = task_input.task_id));

GRANT SELECT, INSERT, UPDATE, DELETE ON tasks.task_input TO veoveo_server;

-- A Task holds at most 64 input requests. A request inserts its input in the statement
-- that locks the Task's row, so concurrent requests for one Task insert one after the
-- other, and each count reads the inputs committed before it.
CREATE FUNCTION tasks.task_input_limit() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF (SELECT count(*) FROM tasks.task_input WHERE task_id = NEW.task_id) >= 64 THEN
        RAISE EXCEPTION 'task % holds 64 input requests', NEW.task_id
            USING ERRCODE = 'check_violation', CONSTRAINT = 'task_input_limit';
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER task_input_limit BEFORE INSERT ON tasks.task_input
FOR EACH ROW EXECUTE FUNCTION tasks.task_input_limit();

-- A step of a Task's work. The owning worker records the step's operation before the
-- step runs and its result once it finishes, so a Task that runs again after recovery
-- skips the steps an earlier run finished and recognizes the ones it left unsettled.
CREATE TABLE tasks.task_step (
    task_id uuid NOT NULL REFERENCES tasks.task (id) ON DELETE CASCADE,
    -- The name the Task's work gives the step on every run.
    step_key text NOT NULL,
    -- Generated when the step is first dispatched and kept across runs, so a provider
    -- call can use it as an idempotency key.
    operation_id uuid NOT NULL,
    dispatched_at timestamptz NOT NULL DEFAULT now(),
    -- SQL NULL means no result yet; JSON null is a present result.
    result jsonb,
    completed_at timestamptz,
    PRIMARY KEY (task_id, step_key),
    CONSTRAINT task_step_key CHECK (
        octet_length(step_key) <= 128 AND tasks.valid_key(step_key)
    ),
    CONSTRAINT task_step_operation_v7 CHECK (
        uuid_extract_version(operation_id) = 7
        AND substr(operation_id::text, 20, 1) IN ('8', '9', 'a', 'b')
    ),
    CONSTRAINT task_step_completion CHECK ((result IS NULL) = (completed_at IS NULL))
);

COMMENT ON TABLE tasks.task_step IS
    'Retention: as long as its Task.';

-- A step keeps its Task, key, operation and dispatch time, and a finished step keeps
-- its result.
CREATE FUNCTION tasks.task_step_settle() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF (NEW.task_id, NEW.step_key, NEW.operation_id, NEW.dispatched_at)
        IS DISTINCT FROM (OLD.task_id, OLD.step_key, OLD.operation_id, OLD.dispatched_at)
        OR OLD.completed_at IS NOT NULL
    THEN
        RAISE EXCEPTION 'step % of task % keeps its operation and its result',
            OLD.step_key, OLD.task_id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER task_step_settle BEFORE UPDATE ON tasks.task_step
FOR EACH ROW EXECUTE FUNCTION tasks.task_step_settle();

ALTER TABLE tasks.task_step ENABLE ROW LEVEL SECURITY;

-- The parent read applies the Task policy, so a server sees only its own steps.
CREATE POLICY task_step_server ON tasks.task_step TO veoveo_server
USING (EXISTS (SELECT FROM tasks.task WHERE task.id = task_step.task_id))
WITH CHECK (EXISTS (SELECT FROM tasks.task WHERE task.id = task_step.task_id));

GRANT SELECT, INSERT, UPDATE, DELETE ON tasks.task_step TO veoveo_server;

-- Outside work a Task hands to a provider and waits for: an operation recorded before its
-- dispatch, the events the provider reports about it, and the Task parked on it while
-- the outcome is pending. A parked Task holds no worker.

CREATE TABLE tasks.task_operation (
    task_id uuid NOT NULL REFERENCES tasks.task (id) ON DELETE CASCADE,
    -- The name the Task's work gives the operation on every run.
    operation_key text NOT NULL,
    -- Generated when the operation is first dispatched and kept across runs, so the
    -- provider call can use it as its idempotency key.
    operation_id uuid NOT NULL UNIQUE,
    -- The Task's server, kept here for the provider-scoped indexes.
    server text NOT NULL,
    -- The provider name the server chose for this kind of outside work.
    provider text NOT NULL,
    -- The provider's identifier of the job, once the dispatch recorded it.
    external_id text,
    state text NOT NULL DEFAULT 'dispatching',
    completion text NOT NULL,
    check_first_ms bigint,
    check_interval_ms bigint,
    check_backoff double precision,
    check_limit integer,
    checks_made integer NOT NULL DEFAULT 0,
    next_check_at timestamptz,
    deadline timestamptz NOT NULL,
    -- SHA-256 of the callback token the dispatch handed to the provider.
    callback_digest bytea,
    -- The final event's payload once the operation settled; SQL NULL until then.
    outcome jsonb,
    dispatched_at timestamptz NOT NULL DEFAULT now(),
    submitted_at timestamptz,
    settled_at timestamptz,
    unresolved_at timestamptz,
    -- The cancellation the server carried to the provider: when it started and the
    -- provider's answer.
    cancel_started_at timestamptz,
    cancel_receipt jsonb,
    cancel_recorded_at timestamptz,
    PRIMARY KEY (task_id, operation_key),
    CONSTRAINT task_operation_key CHECK (
        octet_length(operation_key) <= 128 AND tasks.valid_key(operation_key)
    ),
    CONSTRAINT task_operation_id_v7 CHECK (
        uuid_extract_version(operation_id) = 7
        AND substr(operation_id::text, 20, 1) IN ('8', '9', 'a', 'b')
    ),
    CONSTRAINT task_operation_server CHECK (server ~ '^[a-z][a-z0-9-]{0,47}$'),
    CONSTRAINT task_operation_provider CHECK (provider ~ '^[a-z][a-z0-9._-]{0,63}$'),
    CONSTRAINT task_operation_external CHECK (
        external_id IS NULL OR (
            octet_length(external_id) BETWEEN 1 AND 512
            AND external_id !~ '[\u0001-\u001f\u007f-\u009f]'
        )
    ),
    CONSTRAINT task_operation_state CHECK (
        state IN ('dispatching', 'submitted', 'settled', 'unresolved')
    ),
    CONSTRAINT task_operation_completion CHECK (
        (completion = 'events'
            AND check_first_ms IS NULL AND check_interval_ms IS NULL
            AND check_backoff IS NULL AND check_limit IS NULL AND next_check_at IS NULL)
        OR (completion = 'checks'
            AND check_first_ms >= 0 AND check_interval_ms > 0
            AND check_backoff >= 1 AND check_backoff <= 10
            AND check_limit BETWEEN 1 AND 10000)
    ),
    CONSTRAINT task_operation_checks CHECK (
        checks_made >= 0 AND (check_limit IS NULL OR checks_made <= check_limit)
    ),
    CONSTRAINT task_operation_callback CHECK (
        callback_digest IS NULL OR octet_length(callback_digest) = 32
    ),
    CONSTRAINT task_operation_outcome CHECK (
        (outcome IS NOT NULL) = (state = 'settled')
        AND (settled_at IS NOT NULL) = (state = 'settled')
    ),
    CONSTRAINT task_operation_submitted CHECK (
        (state <> 'dispatching' OR submitted_at IS NULL)
        AND (state <> 'submitted' OR submitted_at IS NOT NULL)
    ),
    CONSTRAINT task_operation_unresolved CHECK (
        state <> 'unresolved' OR unresolved_at IS NOT NULL
    ),
    CONSTRAINT task_operation_cancel CHECK (
        (cancel_receipt IS NULL) = (cancel_recorded_at IS NULL)
        AND (cancel_recorded_at IS NULL OR cancel_started_at IS NOT NULL)
    )
);

COMMENT ON TABLE tasks.task_operation IS
    'Retention: as long as its Task. An unresolved operation pins its Task until an '
    'event settles it.';

-- A provider's own job identifier names one operation of the server.
CREATE UNIQUE INDEX task_operation_external_job ON tasks.task_operation (
    server, provider, external_id
) WHERE external_id IS NOT NULL;

-- The due pass finds pending checks and passed deadlines without a scan.
CREATE INDEX task_operation_check_due ON tasks.task_operation (server, next_check_at)
WHERE next_check_at IS NOT NULL AND state IN ('dispatching', 'submitted');

CREATE INDEX task_operation_deadline ON tasks.task_operation (server, deadline)
WHERE state IN ('dispatching', 'submitted');

-- An operation keeps its Task, key, identity, provider, completion profile and dispatch
-- time, and a settled operation keeps its outcome.
CREATE FUNCTION tasks.task_operation_settle() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF (
        NEW.task_id, NEW.operation_key, NEW.operation_id, NEW.server, NEW.provider,
        NEW.completion, NEW.check_first_ms, NEW.check_interval_ms, NEW.check_backoff,
        NEW.check_limit, NEW.deadline, NEW.dispatched_at
    ) IS DISTINCT FROM (
        OLD.task_id, OLD.operation_key, OLD.operation_id, OLD.server, OLD.provider,
        OLD.completion, OLD.check_first_ms, OLD.check_interval_ms, OLD.check_backoff,
        OLD.check_limit, OLD.deadline, OLD.dispatched_at
    )
        OR (OLD.state = 'settled' AND (
            NEW.state, NEW.outcome, NEW.settled_at
        ) IS DISTINCT FROM (OLD.state, OLD.outcome, OLD.settled_at))
        OR (OLD.external_id IS NOT NULL AND NEW.external_id IS DISTINCT FROM OLD.external_id)
        OR (OLD.callback_digest IS NOT NULL
            AND NEW.callback_digest IS DISTINCT FROM OLD.callback_digest)
        OR (OLD.cancel_receipt IS NOT NULL
            AND NEW.cancel_receipt IS DISTINCT FROM OLD.cancel_receipt)
    THEN
        RAISE EXCEPTION 'operation % of task % keeps its identity and its outcome',
            OLD.operation_key, OLD.task_id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER task_operation_settle BEFORE UPDATE ON tasks.task_operation
FOR EACH ROW EXECUTE FUNCTION tasks.task_operation_settle();

-- What a provider reported about an operation: a webhook delivery or a status check.
CREATE TABLE tasks.task_operation_event (
    server text NOT NULL,
    provider text NOT NULL,
    -- The provider's event identifier, or one the runtime assigns to a status check.
    event_id text NOT NULL,
    task_id uuid NOT NULL,
    operation_key text NOT NULL,
    payload jsonb NOT NULL,
    final boolean NOT NULL,
    -- Whether this event settled its operation. A repeat, a non-final event and a final
    -- event that arrived after another one settled the operation apply nothing.
    applied boolean NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (server, provider, event_id),
    FOREIGN KEY (task_id, operation_key)
        REFERENCES tasks.task_operation (task_id, operation_key) ON DELETE CASCADE,
    CONSTRAINT task_operation_event_id CHECK (tasks.valid_key(event_id)),
    CONSTRAINT task_operation_event_applied CHECK (NOT applied OR final)
);

COMMENT ON TABLE tasks.task_operation_event IS
    'Retention: as long as its operation.';

-- An operation's events are read and removed with it through this index.
CREATE INDEX task_operation_event_operation ON tasks.task_operation_event (
    task_id, operation_key, received_at
);

-- An event is a record of what the provider said; it never changes.
CREATE FUNCTION tasks.task_operation_event_keep() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    RAISE EXCEPTION 'operation events are append-only'
        USING ERRCODE = 'check_violation';
END
$$;

CREATE TRIGGER task_operation_event_keep BEFORE UPDATE ON tasks.task_operation_event
FOR EACH ROW EXECUTE FUNCTION tasks.task_operation_event_keep();

ALTER TABLE tasks.task_operation ENABLE ROW LEVEL SECURITY;
ALTER TABLE tasks.task_operation_event ENABLE ROW LEVEL SECURITY;

-- A server reads and writes the operations of its own Tasks. The parent read applies the
-- Task policy, and the server column must name the Task's server.
CREATE POLICY task_operation_server ON tasks.task_operation TO veoveo_server
USING (server = ANY ((SELECT kernel.current_servers())::text[]))
WITH CHECK (
    server = ANY ((SELECT kernel.current_servers())::text[])
    AND EXISTS (
        SELECT FROM tasks.task
        WHERE task.id = task_operation.task_id AND task.server = task_operation.server
    )
);

CREATE POLICY task_operation_event_server ON tasks.task_operation_event TO veoveo_server
USING (server = ANY ((SELECT kernel.current_servers())::text[]))
WITH CHECK (server = ANY ((SELECT kernel.current_servers())::text[]));

GRANT SELECT, INSERT, UPDATE, DELETE ON tasks.task_operation TO veoveo_server;
GRANT SELECT, INSERT, DELETE ON tasks.task_operation_event TO veoveo_server;
