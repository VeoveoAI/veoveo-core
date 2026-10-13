-- Outside work a Task hands to a provider and waits for: an operation recorded before its
-- dispatch, the events the provider reports about it, and the Task parked on it while
-- the outcome is pending. A parked Task holds no worker.

-- The operation the Task is parked on. A parked Task waits without an owner, so the
-- recovery trigger skips it and a restart leaves it waiting.
ALTER TABLE tasks.task ADD COLUMN parked_on text;

ALTER TABLE tasks.task ADD CONSTRAINT task_parked CHECK (
    parked_on IS NULL
    OR (owner_instance IS NULL AND status IN ('waiting', 'cancel_requested'))
);

-- Recovery applies to a Task that loses its owner while it runs, and never to a Task its
-- worker parked on an operation. The migration replaces the trigger in one transaction.
DROP TRIGGER task_recovery ON tasks.task;

CREATE TRIGGER task_recovery BEFORE UPDATE OF owner_instance ON tasks.task
FOR EACH ROW
WHEN (
    OLD.owner_instance IS NOT NULL
    AND NEW.owner_instance IS NULL
    AND NEW.status IN ('running', 'waiting', 'cancel_requested')
    AND NEW.parked_on IS NULL
)
EXECUTE FUNCTION tasks.task_recovery();

-- Claims take queued Tasks and parked Tasks whose cancellation the server must carry to
-- the provider, oldest first.
DROP INDEX tasks.task_queued;

CREATE INDEX task_claimable ON tasks.task (server, created_at, id)
WHERE status = 'queued' OR (status = 'cancel_requested' AND owner_instance IS NULL);

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

COMMENT ON TABLE tasks.task_operation IS
    'Retention: as long as its Task. An unresolved operation pins its Task until an '
    'event settles it.';

COMMENT ON TABLE tasks.task_operation_event IS
    'Retention: as long as its operation.';
