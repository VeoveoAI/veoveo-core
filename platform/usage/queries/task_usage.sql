-- A Task's cost per kind and currency: its actual records of a kind when it has any,
-- else its estimates of that kind.
WITH task_records AS (
    SELECT kind, phase, currency, quantity, amount
    FROM usage.record
    WHERE tenant = $1 AND task_id = $2
),
chosen AS (
    SELECT r.kind, r.phase, r.currency, r.quantity, r.amount
    FROM task_records AS r
    WHERE r.phase = 'actual' OR NOT EXISTS (
        SELECT FROM task_records AS a WHERE a.kind = r.kind AND a.phase = 'actual'
    )
)
SELECT
    c.kind AS "kind!",
    k.unit AS "unit!",
    c.phase AS "phase!",
    c.currency AS "currency!",
    sum(c.quantity)::text AS "quantity!",
    sum(c.amount)::text AS "amount"
FROM chosen AS c
JOIN usage.kind AS k ON k.kind = c.kind
GROUP BY c.kind, k.unit, c.phase, c.currency
ORDER BY c.kind, c.currency
LIMIT 256
