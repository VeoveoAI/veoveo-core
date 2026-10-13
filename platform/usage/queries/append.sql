-- The installation owner appends usage records in the caller's transaction.
SELECT usage.append($1::jsonb[]) AS "added!"
