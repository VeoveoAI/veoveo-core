-- SET ROLE for the session, with the role as a parameter.
SELECT set_config('role', $1, false) AS "role!"
