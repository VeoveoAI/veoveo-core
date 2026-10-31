-- veoveo: all tenants (the installation-owner fixture holds one Task's row)
-- Lock one Task's row for the rest of the transaction, as a change to it would.
SELECT id AS "id!" FROM tasks.task WHERE id = $1 FOR UPDATE
