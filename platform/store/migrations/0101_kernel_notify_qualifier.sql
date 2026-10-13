-- Row trigger that wakes listeners. The first argument names the key column, the
-- optional second the tenant column, which an empty name omits, and the optional third
-- a qualifier column. Without a qualifier the notification goes to the channel named
-- after the table's schema. With one it goes only to `<schema>.<qualifier>`, such as
-- `tasks.time` for a Task of server `time`; the qualifier column is NOT NULL and keeps
-- its value for the row's life. The payload carries no row data beyond the key and
-- tenant. Replacing the function keeps its grants and the triggers that execute it.
CREATE OR REPLACE FUNCTION kernel.notify_change() RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    changed jsonb := to_jsonb(CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END);
    row_key text := changed ->> TG_ARGV[0];
    channel_name text := TG_TABLE_SCHEMA;
    qualifier text;
BEGIN
    IF TG_NARGS > 2 THEN
        qualifier := changed ->> TG_ARGV[2];
        IF qualifier IS NULL OR qualifier = '' THEN
            RAISE EXCEPTION 'column % of %.% names no notification channel',
                quote_ident(TG_ARGV[2]), TG_TABLE_SCHEMA, TG_TABLE_NAME
                USING ERRCODE = 'null_value_not_allowed';
        END IF;
        channel_name := channel_name || '.' || qualifier;
    END IF;
    PERFORM pg_catalog.pg_notify(channel_name, json_build_object(
        'table', TG_TABLE_NAME,
        'operation', lower(TG_OP),
        'key', CASE WHEN octet_length(row_key) <= 1024 THEN row_key END,
        'tenant', CASE WHEN TG_NARGS > 1 THEN changed ->> TG_ARGV[1] END
    )::text);
    RETURN NULL;
END
$$;
