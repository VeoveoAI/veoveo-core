-- Mark instances whose locks are free and delete those whose locks have stayed free for
-- the grace in $1.
SELECT kernel.reap_instances($1) AS "reaped!"
