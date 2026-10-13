-- Record the calling server's class declarations through its own login.
SELECT audit.declare_server_classes($1::text[], $2::jsonb[])
