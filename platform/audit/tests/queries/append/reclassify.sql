UPDATE audit.record SET occurred_at = occurred_at + interval '1 second' WHERE id = $1
