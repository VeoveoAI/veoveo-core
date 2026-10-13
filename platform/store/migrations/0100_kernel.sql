-- Kernel schema: process instances, singleton locks, change notifications and the
-- roles and schemas of registered servers. The installation owner applies it.

CREATE SCHEMA IF NOT EXISTS kernel;

ALTER DEFAULT PRIVILEGES IN SCHEMA kernel REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC;

-- Every server's group role is a member of veoveo_server, which holds the kernel grants
-- that servers share. Roles belong to the cluster, so another database of the same
-- cluster may already have created it.
DO $$
BEGIN
    IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'veoveo_server') THEN
        CREATE ROLE veoveo_server NOLOGIN;
    END IF;
END
$$;

GRANT USAGE ON SCHEMA kernel TO veoveo_server;

-- A server name is its slug: a lowercase letter followed by at most 47 lowercase
-- letters, digits and hyphens. Its schema replaces each hyphen with an underscore and
-- may not be a kernel schema or a PostgreSQL schema.
CREATE FUNCTION kernel.server_schema(server text) RETURNS text
LANGUAGE plpgsql IMMUTABLE
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    schema_name text := replace(server_schema.server, '-', '_');
BEGIN
    IF server_schema.server IS NULL
        OR server_schema.server !~ '^[a-z][a-z0-9-]{0,47}$'
        OR schema_name LIKE 'pg\_%'
        OR schema_name = ANY (ARRAY[
            'kernel', 'identity', 'tasks', 'audit', 'public', 'information_schema'
        ])
    THEN
        RAISE EXCEPTION 'invalid server name %', quote_nullable(server_schema.server)
            USING ERRCODE = 'invalid_name';
    END IF;
    RETURN schema_name;
END
$$;

-- The slugs of the servers whose group role `member` has the privileges of.
CREATE FUNCTION kernel.servers_of(member name) RETURNS text[]
LANGUAGE sql STABLE
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT coalesce(array_agg(replace(substr(r.rolname, 8), '_', '-') ORDER BY r.rolname), '{}')
    FROM pg_catalog.pg_roles AS r
    WHERE NOT r.rolcanlogin
        AND r.rolname LIKE 'server\_%'
        AND pg_catalog.pg_has_role(r.oid, 'veoveo_server', 'MEMBER')
        AND pg_catalog.pg_has_role(servers_of.member, r.oid, 'USAGE')
$$;

-- The slugs of the servers whose group role the current user belongs to. Row-level
-- security on shared kernel tables admits a row when its server is one of them. A
-- SECURITY DEFINER function runs as its owner, so it asks for the servers of
-- session_user, the calling login, instead.
CREATE FUNCTION kernel.current_servers() RETURNS text[]
LANGUAGE sql STABLE
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT kernel.servers_of(current_user)
$$;

-- Creates the server's group role and its schema, owned by that role. The Gateway calls
-- it when a server registers; repeating it changes nothing.
CREATE FUNCTION kernel.ensure_server(server text) RETURNS void
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    schema_name text := kernel.server_schema(ensure_server.server);
    group_role text := 'server_' || schema_name;
BEGIN
    IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = group_role) THEN
        -- The creating role keeps SET on the new role, so it can give the role a schema.
        PERFORM pg_catalog.set_config('createrole_self_grant', 'set', true);
        BEGIN
            EXECUTE format('CREATE ROLE %I NOLOGIN IN ROLE veoveo_server', group_role);
        EXCEPTION WHEN duplicate_object OR unique_violation THEN
            NULL;
        END;
    END IF;
    BEGIN
        EXECUTE format('CREATE SCHEMA IF NOT EXISTS %I AUTHORIZATION %I', schema_name, group_role);
    EXCEPTION WHEN duplicate_schema OR unique_violation THEN
        NULL;
    END;
END
$$;

-- Creates the login role for one registration of a server: `<schema>_<suffix>`, a
-- member of the server's group role with INHERIT and SET.
CREATE FUNCTION kernel.create_server_login(server text, login text, password text) RETURNS void
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    schema_name text := kernel.server_schema(create_server_login.server);
BEGIN
    IF create_server_login.login IS NULL
        OR create_server_login.login !~ '^[a-z_][a-z0-9_]{0,62}$'
        OR length(create_server_login.login) <= length(schema_name) + 1
        OR left(create_server_login.login, length(schema_name) + 1) <> schema_name || '_'
    THEN
        RAISE EXCEPTION 'login % does not belong to server %',
            quote_nullable(create_server_login.login), create_server_login.server
            USING ERRCODE = 'invalid_name';
    END IF;
    IF create_server_login.password IS NULL OR length(create_server_login.password) < 16 THEN
        RAISE EXCEPTION 'a server login password has at least 16 characters'
            USING ERRCODE = 'invalid_password';
    END IF;
    EXECUTE format(
        'CREATE ROLE %I LOGIN PASSWORD %L IN ROLE %I',
        create_server_login.login, create_server_login.password, 'server_' || schema_name
    );
END
$$;

-- Drops a login that create_server_login created. Dropping an absent login does nothing.
CREATE FUNCTION kernel.drop_server_login(login text) RETURNS void
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    role_oid oid;
BEGIN
    SELECT r.oid INTO role_oid FROM pg_catalog.pg_roles AS r
    WHERE r.rolname = drop_server_login.login;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    IF NOT EXISTS (
        SELECT FROM pg_catalog.pg_roles AS r
        WHERE r.oid = role_oid
            AND r.rolcanlogin
            AND pg_catalog.pg_has_role(r.oid, 'veoveo_server', 'MEMBER')
    ) THEN
        RAISE EXCEPTION 'role % is not a server login', quote_ident(drop_server_login.login)
            USING ERRCODE = 'invalid_name';
    END IF;
    EXECUTE format('DROP ROLE %I', drop_server_login.login);
END
$$;

-- One row per running process that owns work. The process holds a session advisory
-- lock keyed by (1987471726, id) for as long as its connection lives.
CREATE TABLE kernel.instance (
    id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    server text NOT NULL CHECK (server ~ '^[a-z][a-z0-9-]{0,47}$'),
    login name NOT NULL DEFAULT session_user,
    started_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE kernel.instance ENABLE ROW LEVEL SECURITY;

-- The scalar subquery runs once per statement rather than once per row.
CREATE POLICY instance_server ON kernel.instance
    USING (server = ANY ((SELECT kernel.current_servers())::text[]))
    WITH CHECK (server = ANY ((SELECT kernel.current_servers())::text[]) AND login = session_user);

GRANT SELECT, INSERT, DELETE ON kernel.instance TO veoveo_server;

-- Records the calling process as an instance of `server` and takes the lock that proves
-- it alive. The lock is held before the row commits, so a reaper never sees the row
-- without its lock.
CREATE FUNCTION kernel.start_instance(server text) RETURNS integer
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    instance_id integer;
BEGIN
    INSERT INTO kernel.instance (server) VALUES (start_instance.server)
    RETURNING id INTO instance_id;
    PERFORM pg_catalog.pg_advisory_lock(1987471726, instance_id);
    RETURN instance_id;
END
$$;

-- Whether some session of this database holds the instance's lock.
CREATE FUNCTION kernel.instance_alive(instance integer) RETURNS boolean
LANGUAGE sql
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT EXISTS (
        SELECT FROM pg_catalog.pg_locks AS l
        WHERE l.locktype = 'advisory'
            AND l.database = (
                SELECT d.oid FROM pg_catalog.pg_database AS d
                WHERE d.datname = pg_catalog.current_database()
            )
            AND l.classid = 1987471726::oid
            AND l.objid = instance_alive.instance::oid
            AND l.objsubid = 2
            AND l.granted
    )
$$;

-- Deletes the rows of instances whose locks are free and returns how many it deleted.
-- Work that references an instance through ON DELETE SET NULL loses its owner in the
-- same transaction.
CREATE FUNCTION kernel.reap_instances() RETURNS integer
LANGUAGE sql SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
    WITH reaped AS (
        DELETE FROM kernel.instance AS i
        WHERE NOT kernel.instance_alive(i.id)
        RETURNING 1
    )
    SELECT count(*)::integer FROM reaped
$$;

-- Blocks until this session holds the singleton lock keyed by (1987474279, hash of the
-- name). The lock lives until the session ends.
CREATE FUNCTION kernel.lock_singleton(singleton text) RETURNS void
LANGUAGE sql
SET search_path = pg_catalog, pg_temp
AS $$
    SELECT pg_catalog.pg_advisory_lock(1987474279, pg_catalog.hashtext(lock_singleton.singleton))
$$;

-- Row trigger that wakes listeners of the table's schema channel. The first argument
-- names the key column and the optional second the tenant column. The payload carries
-- no row data beyond them.
CREATE FUNCTION kernel.notify_change() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    changed jsonb := to_jsonb(CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END);
    row_key text := changed ->> TG_ARGV[0];
BEGIN
    PERFORM pg_catalog.pg_notify(TG_TABLE_SCHEMA, json_build_object(
        'table', TG_TABLE_NAME,
        'operation', lower(TG_OP),
        'key', CASE WHEN octet_length(row_key) <= 1024 THEN row_key END,
        'tenant', CASE WHEN TG_NARGS > 1 THEN changed ->> TG_ARGV[1] END
    )::text);
    RETURN NULL;
END
$$;

GRANT EXECUTE ON FUNCTION
    kernel.servers_of(name),
    kernel.current_servers(),
    kernel.start_instance(text),
    kernel.instance_alive(integer),
    kernel.reap_instances(),
    kernel.lock_singleton(text),
    kernel.notify_change()
TO veoveo_server;
