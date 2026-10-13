-- A page plans with its actual filters, so an actor, trace or time filter seeks its index
-- even after the connection has run the statement many times.
SET LOCAL plan_cache_mode = force_custom_plan
