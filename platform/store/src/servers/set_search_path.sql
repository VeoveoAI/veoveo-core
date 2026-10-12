-- SET search_path for the session, with the schema as a parameter.
SELECT set_config('search_path', $1, false) AS "search_path!"
