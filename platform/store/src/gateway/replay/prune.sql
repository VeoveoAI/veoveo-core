DELETE FROM gateway.replay_id
WHERE kind = $1 AND expires_at <= $2
