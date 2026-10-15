-- Identity schema: the installation's enterprise, its tenants, their principals and
-- Work Contexts. Registered servers record identities through identity.ensure_identity.

CREATE SCHEMA identity;

ALTER DEFAULT PRIVILEGES IN SCHEMA identity REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC;

GRANT USAGE ON SCHEMA identity TO veoveo_server;

-- Identifying text: not blank, without ASCII control characters and at most
-- `max_bytes` bytes long.
CREATE FUNCTION identity.valid_text(value text, max_bytes integer) RETURNS boolean
LANGUAGE sql IMMUTABLE STRICT
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT btrim(valid_text.value) <> ''
        AND octet_length(valid_text.value) <= valid_text.max_bytes
        AND valid_text.value !~ '[[:cntrl:]]'
$$;

CREATE TABLE identity.enterprise (
    id uuid PRIMARY KEY,
    slug text NOT NULL UNIQUE CHECK (identity.valid_text(slug, 256)),
    name text NOT NULL CHECK (identity.valid_text(name, 512)),
    enabled boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE identity.enterprise IS
    'Retention: for the life of the installation.';

CREATE TABLE identity.tenant (
    id uuid PRIMARY KEY,
    enterprise uuid NOT NULL REFERENCES identity.enterprise (id),
    slug text NOT NULL CHECK (identity.valid_text(slug, 256)),
    name text NOT NULL CHECK (identity.valid_text(name, 512)),
    enabled boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (enterprise, slug)
);

COMMENT ON TABLE identity.tenant IS
    'Retention: for the life of the installation.';

CREATE TABLE identity.principal (
    id uuid PRIMARY KEY,
    tenant uuid NOT NULL REFERENCES identity.tenant (id),
    principal_key text NOT NULL CHECK (identity.valid_text(principal_key, 512)),
    kind text NOT NULL CHECK (kind IN ('user', 'service')),
    issuer text NOT NULL CHECK (identity.valid_text(issuer, 2048)),
    subject text NOT NULL CHECK (identity.valid_text(subject, 2048)),
    display_name text NOT NULL CHECK (identity.valid_text(display_name, 512)),
    enabled boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant, principal_key),
    UNIQUE (tenant, issuer, subject)
);

COMMENT ON TABLE identity.principal IS
    'Retention: for the life of the installation.';

-- A Work Context and its output policy, as the active control-plane revision defines it.
CREATE TABLE identity.work_context (
    id uuid PRIMARY KEY,
    tenant uuid NOT NULL REFERENCES identity.tenant (id),
    context_key text NOT NULL CHECK (identity.valid_text(context_key, 256)),
    title text NOT NULL CHECK (identity.valid_text(title, 512)),
    policy_revision text NOT NULL CHECK (identity.valid_text(policy_revision, 256)),
    owner_kind text NOT NULL CHECK (owner_kind IN ('principal', 'group')),
    owner_key text NOT NULL CHECK (identity.valid_text(owner_key, 512)),
    classification text CHECK (identity.valid_text(classification, 256)),
    data_labels text[] NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant, context_key)
);

COMMENT ON TABLE identity.work_context IS
    'Retention: while the active control-plane revision defines it; a publication '
    'deletes every Work Context the new revision omits.';

-- The grants a Work Context gives on its outputs, in declaration order.
CREATE TABLE identity.work_context_grant (
    work_context uuid NOT NULL REFERENCES identity.work_context (id) ON DELETE CASCADE,
    ordinal integer NOT NULL CHECK (ordinal >= 0),
    subject_kind text NOT NULL CHECK (subject_kind IN ('principal', 'group')),
    subject_key text NOT NULL CHECK (identity.valid_text(subject_key, 512)),
    permission text NOT NULL CHECK (permission IN ('read', 'write', 'admin')),
    PRIMARY KEY (work_context, ordinal)
);

COMMENT ON TABLE identity.work_context_grant IS
    'Retention: until a publication replaces its Work Context''s grants, or deletes '
    'the Work Context.';

-- The membership rules of a Work Context, in declaration order.
CREATE TABLE identity.work_context_membership (
    work_context uuid NOT NULL REFERENCES identity.work_context (id) ON DELETE CASCADE,
    ordinal integer NOT NULL CHECK (ordinal >= 0),
    level text NOT NULL CHECK (level IN ('viewer', 'contributor', 'custodian', 'owner')),
    principals text[] NOT NULL DEFAULT '{}',
    groups text[] NOT NULL DEFAULT '{}',
    roles text[] NOT NULL DEFAULT '{}',
    oauth_clients text[] NOT NULL DEFAULT '{}',
    PRIMARY KEY (work_context, ordinal)
);

COMMENT ON TABLE identity.work_context_membership IS
    'Retention: until a publication replaces its Work Context''s membership rules, or '
    'deletes the Work Context.';

-- Records the installation enterprise, a tenant and a principal, each keyed by its
-- deterministic identifier. Existing rows keep their security fields: a stored tenant or
-- principal that disagrees with the given keys raises SQLSTATE VV001 or VV002. A given
-- display name replaces the stored one; without one, a new principal takes the fallback.
--
-- Each row also has a unique natural key that ON CONFLICT (id) does not arbitrate, so
-- two first-time inserts of the same row would race on it. Creating a missing row
-- therefore first takes the transaction advisory lock (1987476603, hash of its
-- identifier): concurrent first calls create the row once, and the later caller's
-- insert finds the committed row. A call for rows that exist takes no lock.
CREATE FUNCTION identity.ensure_identity(
    enterprise uuid,
    tenant uuid,
    tenant_key text,
    principal uuid,
    principal_key text,
    kind text,
    issuer text,
    subject text,
    display_name text,
    fallback_display_name text
) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT FROM identity.enterprise AS e WHERE e.id = ensure_identity.enterprise
    ) THEN
        PERFORM pg_catalog.pg_advisory_xact_lock(
            1987476603, pg_catalog.hashtext(ensure_identity.enterprise::text)
        );
        INSERT INTO identity.enterprise (id, slug, name)
        VALUES (ensure_identity.enterprise, 'installation', 'Veoveo installation')
        ON CONFLICT (id) DO NOTHING;
    END IF;

    IF NOT EXISTS (SELECT FROM identity.tenant AS t WHERE t.id = ensure_identity.tenant) THEN
        PERFORM pg_catalog.pg_advisory_xact_lock(
            1987476603, pg_catalog.hashtext(ensure_identity.tenant::text)
        );
        INSERT INTO identity.tenant (id, enterprise, slug, name)
        VALUES (
            ensure_identity.tenant,
            ensure_identity.enterprise,
            ensure_identity.tenant_key,
            ensure_identity.tenant_key
        )
        ON CONFLICT (id) DO NOTHING;
    END IF;
    PERFORM FROM identity.tenant AS t
    WHERE t.id = ensure_identity.tenant
        AND (t.enterprise <> ensure_identity.enterprise OR t.slug <> ensure_identity.tenant_key);
    IF FOUND THEN
        RAISE EXCEPTION 'stored tenant conflicts with key %', ensure_identity.tenant_key
            USING ERRCODE = 'VV001';
    END IF;

    IF NOT EXISTS (
        SELECT FROM identity.principal AS p WHERE p.id = ensure_identity.principal
    ) THEN
        PERFORM pg_catalog.pg_advisory_xact_lock(
            1987476603, pg_catalog.hashtext(ensure_identity.principal::text)
        );
    END IF;
    INSERT INTO identity.principal AS p
        (id, tenant, principal_key, kind, issuer, subject, display_name)
    VALUES (
        ensure_identity.principal,
        ensure_identity.tenant,
        ensure_identity.principal_key,
        ensure_identity.kind,
        ensure_identity.issuer,
        ensure_identity.subject,
        coalesce(ensure_identity.display_name, ensure_identity.fallback_display_name)
    )
    ON CONFLICT (id) DO UPDATE
        SET display_name = excluded.display_name, updated_at = now()
        WHERE ensure_identity.display_name IS NOT NULL
            AND p.display_name <> excluded.display_name;
    PERFORM FROM identity.principal AS p
    WHERE p.id = ensure_identity.principal
        AND (p.tenant <> ensure_identity.tenant
            OR p.principal_key <> ensure_identity.principal_key
            OR p.kind <> ensure_identity.kind
            OR p.issuer <> ensure_identity.issuer
            OR p.subject <> ensure_identity.subject);
    IF FOUND THEN
        RAISE EXCEPTION 'stored principal conflicts with key %', ensure_identity.principal_key
            USING ERRCODE = 'VV002';
    END IF;
END
$$;

GRANT EXECUTE ON FUNCTION
    identity.ensure_identity(uuid, uuid, text, uuid, text, text, text, text, text, text)
TO veoveo_server;
