-- veoveo: all tenants (the installation-owner fixture attempts to rebind one Task)
UPDATE tasks.task SET deployment = $2 WHERE id = $1
