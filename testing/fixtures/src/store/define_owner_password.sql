-- A session function that sets the owner's password from a parameter.
CREATE FUNCTION pg_temp.set_owner_password(password text) RETURNS void
LANGUAGE plpgsql
AS $$
BEGIN
    EXECUTE format('ALTER ROLE veoveo_owner PASSWORD %L', set_owner_password.password);
END
$$
