-- A registered server appends its own usage records in the caller's transaction.
SELECT usage.append_server_records($1::jsonb[]) AS "added!"
