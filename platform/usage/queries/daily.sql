-- A tenant's daily totals over [$2, $3), in day, principal, kind, phase and currency order.
SELECT
    day AS "day!",
    principal AS "principal!",
    kind AS "kind!",
    phase AS "phase!",
    currency AS "currency!",
    quantity::text AS "quantity!",
    amount::text AS "amount!",
    records AS "records!"
FROM usage.daily
WHERE tenant = $1 AND day >= $2 AND day < $3
ORDER BY day, principal, source, kind, phase, currency
LIMIT $4
