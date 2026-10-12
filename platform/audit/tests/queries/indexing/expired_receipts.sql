-- Receipts recorded two days ago, past their one-day retention.
INSERT INTO audit.indexing_receipt (partition, record, fingerprint, recorded_at)
SELECT
    'tenant:test-tenant',
    gen_random_uuid(),
    'sha256:' || repeat('0', 64),
    now() - interval '2 days'
FROM generate_series(1, $1::integer)
