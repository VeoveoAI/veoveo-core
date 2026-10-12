SELECT
    generation,
    consumed_at,
    replay_detected_at,
    delivery_envelope IS NOT NULL AS "delivered!"
FROM gateway.refresh_token
ORDER BY generation
