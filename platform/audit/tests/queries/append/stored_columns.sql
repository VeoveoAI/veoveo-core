-- The columns the database generated or assigned for one record.
SELECT
    r.partition,
    r.class,
    r.outcome,
    r.actor,
    r.trace_id,
    r.day,
    r.recorded_at,
    r.txid::text AS "txid!"
FROM audit.record AS r
WHERE r.id = $1
