-- veoveo: unbounded (a publication replaces the membership rules of one Work Context)
DELETE FROM identity.work_context_membership
WHERE work_context = $1
