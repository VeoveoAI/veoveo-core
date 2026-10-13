-- The lookup the ON DELETE SET NULL action of refresh_token.replacement runs.
EXPLAIN (FORMAT JSON)
SELECT id FROM gateway.refresh_token WHERE replacement = $1
