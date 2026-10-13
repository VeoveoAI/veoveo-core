-- The installation owner creates the monthly record partitions through a month.
SELECT usage.prepare_partitions($1) AS "created!"
