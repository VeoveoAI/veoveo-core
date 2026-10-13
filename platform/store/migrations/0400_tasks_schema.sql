-- Tasks schema: durable Tasks and the inputs they request.

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
