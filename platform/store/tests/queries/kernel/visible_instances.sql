SELECT id AS "id!: InstanceId", server, login::text AS "login!"
FROM kernel.instance
ORDER BY id
