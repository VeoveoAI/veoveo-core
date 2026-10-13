-- A registered server declares its own usage kinds through its login.
SELECT usage.declare_server_kinds($1::text[], $2::text[], $3::text[], $4::boolean[]) AS "declared"
