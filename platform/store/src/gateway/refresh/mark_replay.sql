UPDATE gateway.refresh_token
SET replay_detected_at = coalesce(replay_detected_at, $2)
WHERE id = $1
