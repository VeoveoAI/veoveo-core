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
    -- The surface the owner submitted through, such as a Gateway profile. The owner
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
    CONSTRAINT task_revision CHECK (revision >= 1)
);

-- A repeated creation with one key returns the existing Task; pruning releases the key.
CREATE UNIQUE INDEX task_idempotency ON tasks.task (
    server, tenant, owner_principal, surface, idempotency_key
) WHERE idempotency_key IS NOT NULL;

-- Owner reads, pages and subscriptions.
CREATE INDEX task_owner_page ON tasks.task (
    server, tenant, owner_principal, surface, created_at, id
);

-- Claims take the oldest queued Task of a server.
CREATE INDEX task_queued ON tasks.task (server, created_at, id)
WHERE status = 'queued';

-- Reaping an instance clears ownership through this index.
CREATE INDEX task_owner_instance ON tasks.task (owner_instance)
WHERE owner_instance IS NOT NULL;

CREATE INDEX task_retention ON tasks.task (server, retention_expires_at)
WHERE status IN ('succeeded', 'failed', 'cancelled') AND retention_pins = '{}';

ALTER TABLE tasks.task ENABLE ROW LEVEL SECURITY;

-- A registered server reads and writes the Tasks of its own servers. The scalar
-- subquery runs once per statement.
CREATE POLICY task_server ON tasks.task TO veoveo_server
USING (server = ANY ((SELECT kernel.current_servers())::text[]))
WITH CHECK (server = ANY ((SELECT kernel.current_servers())::text[]));

GRANT SELECT, INSERT, UPDATE, DELETE ON tasks.task TO veoveo_server;
