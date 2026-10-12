-- One page of a partition's daily counts, newest day first, then by class and outcome.
SELECT
    d.day AS "day!",
    d.class AS "class!: AuditClass",
    d.outcome AS "outcome!: AuditOutcome",
    d.count AS "count!"
FROM audit.daily AS d
WHERE d.partition = $1
    AND d.day >= $2
    AND d.day < $3
    AND (
        $4::timestamptz IS NULL
        OR d.day < $4
        OR (d.day = $4 AND d.class > $5)
        OR (d.day = $4 AND d.class = $5 AND d.outcome > $6)
    )
ORDER BY d.day DESC, d.class, d.outcome
LIMIT $7
