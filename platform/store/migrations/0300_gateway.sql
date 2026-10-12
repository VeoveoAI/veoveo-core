-- Gateway schema: control-plane revisions, hosted OAuth state, server registrations
-- and Task routes. The Gateway connects as the installation owner and is the only
-- writer; registered servers receive no grant on this schema.

CREATE SCHEMA gateway;

ALTER DEFAULT PRIVILEGES IN SCHEMA gateway REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC;

-- Rejects every change to a row of an append-only table.
CREATE FUNCTION gateway.reject_change() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    RAISE EXCEPTION '%.% rows are immutable', TG_TABLE_SCHEMA, TG_TABLE_NAME
        USING ERRCODE = 'restrict_violation';
END
$$;

-- One published installation configuration. A revision never changes once written.
CREATE TABLE gateway.control_revision (
    id text PRIMARY KEY CHECK (id <> ''),
    digest text NOT NULL CHECK (digest ~ '^sha256:[0-9a-f]{64}$'),
    source text NOT NULL CHECK (source IN ('admin_api', 'seed_file')),
    applied_at timestamptz NOT NULL,
    applied_by text NOT NULL CHECK (applied_by <> ''),
    tenant text CHECK (tenant <> ''),
    control_plane jsonb NOT NULL CHECK (jsonb_typeof(control_plane) = 'object')
);

CREATE TRIGGER control_revision_immutable
    BEFORE UPDATE ON gateway.control_revision
    FOR EACH ROW EXECUTE FUNCTION gateway.reject_change();

-- The objects of one revision, one row per configuration object. A profile row names
-- the policy version it evaluates under.
CREATE TABLE gateway.control_object (
    revision text NOT NULL REFERENCES gateway.control_revision (id),
    kind text NOT NULL CHECK (kind ~ '^[A-Za-z0-9_.:/-]{1,128}$'),
    object_id text NOT NULL CHECK (object_id <> ''),
    tenant text CHECK (tenant <> ''),
    document jsonb NOT NULL CHECK (jsonb_typeof(document) = 'object'),
    profile_policy_version text CHECK (
        profile_policy_version <> '' AND profile_policy_version !~ '[[:space:][:cntrl:]]'
    ),
    PRIMARY KEY (revision, kind, object_id),
    CHECK ((kind = 'profile') = (profile_policy_version IS NOT NULL))
);

CREATE TRIGGER control_object_immutable
    BEFORE UPDATE ON gateway.control_object
    FOR EACH ROW EXECUTE FUNCTION gateway.reject_change();

-- The pointer to the active revision. The table holds exactly one row; its revision is
-- null until the first publication. A publication locks the row, so publications
-- serialize across replicas.
CREATE TABLE gateway.control_active (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    revision text REFERENCES gateway.control_revision (id),
    updated_at timestamptz NOT NULL DEFAULT now()
);

INSERT INTO gateway.control_active (singleton) VALUES (true);

CREATE TRIGGER control_active_singleton
    BEFORE INSERT OR DELETE ON gateway.control_active
    FOR EACH STATEMENT EXECUTE FUNCTION gateway.reject_change();

-- A revoked JWT, kept until the token itself expires.
CREATE TABLE gateway.jwt_revocation (
    profile text NOT NULL CHECK (profile <> ''),
    issuer text NOT NULL CHECK (issuer <> ''),
    jwt_id text NOT NULL CHECK (jwt_id <> ''),
    revoked_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    reason text,
    PRIMARY KEY (profile, issuer, jwt_id)
);

CREATE INDEX jwt_revocation_expiry ON gateway.jwt_revocation (expires_at);

-- The `jti` of a client assertion or ID-JAG seen before it expires. A live row refuses
-- the same identifier; an expired row may be claimed again.
CREATE TABLE gateway.replay_id (
    kind text NOT NULL CHECK (kind IN ('client_assertion', 'id_jag')),
    authorization_server text NOT NULL CHECK (authorization_server <> ''),
    client_id text NOT NULL CHECK (client_id <> ''),
    jwt_id text NOT NULL CHECK (jwt_id <> ''),
    seen_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    PRIMARY KEY (kind, authorization_server, client_id, jwt_id)
);

CREATE INDEX replay_id_expiry ON gateway.replay_id (kind, expires_at);

-- A hosted authorization request waiting for the identity provider's callback. The
-- callback consumes it by deleting it.
CREATE TABLE gateway.authorization_request (
    idp_state text PRIMARY KEY CHECK (idp_state <> ''),
    profile text NOT NULL CHECK (profile <> ''),
    oauth_client_id text NOT NULL CHECK (oauth_client_id <> ''),
    work_context text NOT NULL CHECK (work_context <> ''),
    oidc_client text NOT NULL CHECK (oidc_client <> ''),
    redirect_uri text NOT NULL CHECK (redirect_uri <> ''),
    client_state text,
    requested_scopes text[] NOT NULL,
    code_challenge text NOT NULL,
    code_challenge_method text NOT NULL CHECK (code_challenge_method = 'S256'),
    idp_code_verifier text NOT NULL,
    idp_code_challenge text NOT NULL,
    idp_code_challenge_method text NOT NULL CHECK (idp_code_challenge_method = 'S256'),
    nonce text NOT NULL CHECK (nonce <> ''),
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    CHECK (created_at < expires_at)
);

CREATE INDEX authorization_request_expiry ON gateway.authorization_request (expires_at);

-- An issued authorization code and the principal it was issued to. Redemption sets
-- consumed_at once.
CREATE TABLE gateway.authorization_code (
    code text PRIMARY KEY CHECK (code <> ''),
    profile text NOT NULL CHECK (profile <> ''),
    oauth_client_id text NOT NULL CHECK (oauth_client_id <> ''),
    work_context text NOT NULL CHECK (work_context <> ''),
    oidc_client text NOT NULL CHECK (oidc_client <> ''),
    redirect_uri text NOT NULL CHECK (redirect_uri <> ''),
    client_state text,
    scopes text[] NOT NULL,
    code_challenge text NOT NULL,
    code_challenge_method text NOT NULL CHECK (code_challenge_method = 'S256'),
    principal jsonb NOT NULL CHECK (jsonb_typeof(principal) = 'object'),
    principal_display_name text NOT NULL CHECK (principal_display_name <> ''),
    issued_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    CHECK (issued_at < expires_at)
);

CREATE INDEX authorization_code_expiry ON gateway.authorization_code (expires_at);

-- A rotating refresh-token family. Access tokens bound to the family stay valid while
-- it is unrevoked and unexpired.
CREATE TABLE gateway.refresh_family (
    id uuid PRIMARY KEY,
    authorization_server text NOT NULL CHECK (authorization_server <> ''),
    profile text NOT NULL CHECK (profile <> ''),
    oauth_client_id text NOT NULL CHECK (oauth_client_id <> ''),
    work_context text NOT NULL CHECK (work_context <> ''),
    principal_id text NOT NULL CHECK (principal_id <> ''),
    tenant text CHECK (tenant <> ''),
    scopes text[] NOT NULL,
    principal jsonb NOT NULL CHECK (jsonb_typeof(principal) = 'object'),
    principal_display_name text NOT NULL CHECK (principal_display_name <> ''),
    current_generation bigint NOT NULL CHECK (current_generation >= 0),
    issued_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    revocation_reason text CHECK (revocation_reason IN ('client_revocation', 'token_replay')),
    CHECK (issued_at < expires_at),
    CHECK ((revoked_at IS NULL) = (revocation_reason IS NULL))
);

CREATE INDEX refresh_family_expiry ON gateway.refresh_family (expires_at);

-- One generation of a family. The token itself is never stored, only its digest. A
-- successor carries an encrypted delivery envelope for a short window, so a concurrent
-- presentation of its predecessor receives the same successor.
CREATE TABLE gateway.refresh_token (
    id uuid PRIMARY KEY,
    family uuid NOT NULL REFERENCES gateway.refresh_family (id) ON DELETE CASCADE,
    token_digest text NOT NULL UNIQUE CHECK (token_digest ~ '^sha256:[0-9a-f]{64}$'),
    generation bigint NOT NULL CHECK (generation >= 0),
    issued_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    replacement uuid REFERENCES gateway.refresh_token (id) ON DELETE SET NULL,
    replay_detected_at timestamptz,
    delivery_envelope text CHECK (delivery_envelope <> ''),
    delivery_expires_at timestamptz,
    UNIQUE (family, generation),
    CHECK (issued_at < expires_at),
    CHECK ((delivery_envelope IS NULL) = (delivery_expires_at IS NULL)),
    CHECK (replacement IS NULL OR consumed_at IS NOT NULL)
);

CREATE INDEX refresh_token_expiry ON gateway.refresh_token (expires_at);
CREATE INDEX refresh_token_delivery_expiry ON gateway.refresh_token (delivery_expires_at)
    WHERE delivery_expires_at IS NOT NULL;

-- The first registration of a server pins its namespace to the registering key's RFC
-- 7638 thumbprint. Releasing a namespace is an administrative action.
CREATE TABLE gateway.namespace_claim (
    server text PRIMARY KEY CHECK (server ~ '^[a-z][a-z0-9-]{0,47}$'),
    key_thumbprint text NOT NULL CHECK (key_thumbprint ~ '^[A-Za-z0-9_-]{43}$'),
    claimed_at timestamptz NOT NULL
);

-- One RFC 7591 registration and the PostgreSQL login it received. live_at is the last
-- time the Gateway saw a live kernel instance of that login; a registration that stays
-- without one past the grace period is removed together with its login.
CREATE TABLE gateway.registration (
    client_id text PRIMARY KEY CHECK (client_id <> ''),
    server text NOT NULL REFERENCES gateway.namespace_claim (server),
    metadata jsonb NOT NULL CHECK (jsonb_typeof(metadata) = 'object'),
    software_statement text NOT NULL CHECK (software_statement <> ''),
    access_token_digest text NOT NULL CHECK (access_token_digest ~ '^sha256:[0-9a-f]{64}$'),
    login text NOT NULL UNIQUE CHECK (login ~ '^[a-z_][a-z0-9_]{0,62}$'),
    issued_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    live_at timestamptz NOT NULL,
    CHECK (metadata ->> 'software_id' = server)
);

CREATE INDEX registration_server ON gateway.registration (server);

-- Replicas follow registrations on the `gateway` channel. Liveness stamps do not wake
-- them.
CREATE TRIGGER registration_change
    AFTER INSERT OR DELETE OR UPDATE OF metadata, software_statement ON gateway.registration
    FOR EACH ROW EXECUTE FUNCTION kernel.notify_change('client_id');

-- The public identifier of an upstream Task the Gateway projected. The source tuple
-- (server, upstream Task, owner, profile) is unique, so a retried projection finds the
-- original route with its original authority and expiry. The owner columns record the
-- actor and provenance that created the Task; later access must present the same ones
-- and retain the recorded data labels.
CREATE TABLE gateway.task_route (
    id text PRIMARY KEY CHECK (id ~ '^gtr_[A-Za-z0-9_-]{43}$'),
    tenant uuid NOT NULL REFERENCES identity.tenant (id),
    owner uuid NOT NULL REFERENCES identity.principal (id),
    work_context uuid NOT NULL REFERENCES identity.work_context (id) ON DELETE CASCADE,
    profile text NOT NULL CHECK (profile <> ''),
    server text NOT NULL CHECK (server <> ''),
    source_task_id text NOT NULL CHECK (octet_length(source_task_id) BETWEEN 1 AND 512),
    authority_digest text NOT NULL CHECK (authority_digest ~ '^sha256:[0-9a-f]{64}$'),
    owner_kind text NOT NULL CHECK (owner_kind IN ('user', 'service')),
    owner_issuer text NOT NULL CHECK (owner_issuer <> ''),
    owner_subject text NOT NULL CHECK (owner_subject <> ''),
    invocation_mode text NOT NULL CHECK (invocation_mode IN ('direct', 'delegated', 'automated')),
    initiator text CHECK (initiator <> ''),
    delegation_id text CHECK (delegation_id <> ''),
    data_labels text[] NOT NULL,
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    UNIQUE (server, source_task_id, owner, profile),
    CHECK (created_at < expires_at),
    CHECK ((invocation_mode = 'automated') = (initiator IS NULL)),
    CHECK ((invocation_mode = 'delegated') = (delegation_id IS NOT NULL))
);

-- The digest that binds an upload to its Work Context's current authority: the policy
-- revision, the output policy and the membership rules.
CREATE FUNCTION gateway.work_context_digest(work_context uuid) RETURNS text
LANGUAGE sql STABLE
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT 'sha256:' || encode(sha256(convert_to(jsonb_build_array(
        wc.policy_revision,
        jsonb_build_object(
            'owner_kind', wc.owner_kind,
            'owner_key', wc.owner_key,
            'classification', wc.classification,
            'data_labels', to_jsonb(wc.data_labels),
            'initial_grants', coalesce((
                SELECT jsonb_agg(jsonb_build_array(g.subject_kind, g.subject_key, g.permission)
                    ORDER BY g.ordinal)
                FROM identity.work_context_grant AS g
                WHERE g.work_context = wc.id
            ), '[]'::jsonb)
        ),
        coalesce((
            SELECT jsonb_agg(jsonb_build_array(
                m.level, to_jsonb(m.principals), to_jsonb(m.groups), to_jsonb(m.roles),
                to_jsonb(m.oauth_clients)
            ) ORDER BY m.ordinal)
            FROM identity.work_context_membership AS m
            WHERE m.work_context = wc.id
        ), '[]'::jsonb)
    )::text, 'UTF8')), 'hex')
    FROM identity.work_context AS wc
    WHERE wc.id = work_context_digest.work_context
$$;

-- The digest of a profile and the policy set it evaluates under, in the active revision.
-- Null when the active revision lacks exactly one of either.
CREATE FUNCTION gateway.profile_policy_digest(profile text) RETURNS text
LANGUAGE sql STABLE
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT 'sha256:' || encode(sha256(convert_to(
        jsonb_build_array(p.document, policy.document)::text, 'UTF8'
    )), 'hex')
    FROM gateway.control_active AS a
    JOIN gateway.control_object AS p
        ON p.revision = a.revision AND p.kind = 'profile' AND p.object_id = profile_policy_digest.profile
    JOIN gateway.control_object AS policy
        ON policy.revision = a.revision AND policy.kind = 'policy'
            AND policy.object_id = p.profile_policy_version
$$;

-- The versions an Artifact upload binds to: the active configuration's digest, the Work
-- Context's policy revision and digest, and the profile's policy digest. No row when
-- the Work Context or the active revision is missing.
CREATE FUNCTION gateway.artifact_upload_authority(work_context uuid, profile text)
RETURNS TABLE (
    control_plane_digest text,
    policy_revision text,
    context_digest text,
    profile_policy_digest text
)
LANGUAGE sql STABLE
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT
        r.digest,
        wc.policy_revision,
        gateway.work_context_digest(wc.id),
        gateway.profile_policy_digest(artifact_upload_authority.profile)
    FROM gateway.control_active AS a
    JOIN gateway.control_revision AS r ON r.id = a.revision
    CROSS JOIN identity.work_context AS wc
    WHERE wc.id = artifact_upload_authority.work_context
$$;
