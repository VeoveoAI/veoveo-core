-- Give the current transaction its transaction ID before it writes anything.
SELECT pg_current_xact_id()::text AS "txid!"
