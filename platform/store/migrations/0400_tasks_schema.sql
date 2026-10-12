-- The tasks schema holds durable MCP Tasks and their input exchanges.
CREATE SCHEMA tasks;
GRANT USAGE ON SCHEMA tasks TO veoveo_server;

-- A Task key, retention pin, input method or idempotency key: 1 to 256 UTF-8 bytes
-- without a control character (Unicode category Cc).
CREATE FUNCTION tasks.is_valid_key(value text) RETURNS boolean
LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE
RETURN octet_length(value) BETWEEN 1 AND 256 AND value !~ '[\u0001-\u001f\u007f-\u009f]';

-- Retention pins are distinct valid keys.
CREATE FUNCTION tasks.is_valid_pin_set(pins text[]) RETURNS boolean
LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE
BEGIN ATOMIC
    SELECT coalesce(bool_and(tasks.is_valid_key(pin)), true)
        AND count(*) = count(DISTINCT pin)
    FROM unnest(pins) AS pin;
END;
