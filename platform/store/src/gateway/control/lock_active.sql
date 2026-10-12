-- Publications serialize on the single pointer row.
SELECT revision
FROM gateway.control_active
FOR UPDATE
