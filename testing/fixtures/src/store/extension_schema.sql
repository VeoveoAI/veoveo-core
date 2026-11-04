-- The installation bootstrap's extension schema: the superuser owns it, every role may
-- use the extensions in it and none may create anything there.
CREATE SCHEMA IF NOT EXISTS _extensions AUTHORIZATION postgres;
REVOKE ALL ON SCHEMA _extensions FROM PUBLIC;
GRANT USAGE ON SCHEMA _extensions TO PUBLIC;
