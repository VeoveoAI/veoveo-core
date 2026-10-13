-- veoveo: unbounded (a publication replaces the grants of one Work Context)
DELETE FROM identity.work_context_grant
WHERE work_context = $1
