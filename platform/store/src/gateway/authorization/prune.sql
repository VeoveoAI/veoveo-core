WITH requests AS (
    DELETE FROM gateway.authorization_request WHERE expires_at <= $1 RETURNING 1
), codes AS (
    DELETE FROM gateway.authorization_code WHERE expires_at <= $1 RETURNING 1
)
SELECT (SELECT count(*) FROM requests) + (SELECT count(*) FROM codes) AS "deleted!"
