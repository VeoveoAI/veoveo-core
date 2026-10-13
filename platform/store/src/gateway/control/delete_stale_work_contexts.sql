-- Work Contexts the new revision no longer defines. Their grants, membership rules and
-- Task routes go with them.
-- veoveo: all tenants (a publication replaces the Work Contexts of every tenant)
-- veoveo: unbounded (the publication removes every omitted Work Context in its own transaction)
-- veoveo: scan (finding the omitted Work Contexts reads every Work Context once per publication)
DELETE FROM identity.work_context
WHERE NOT (id = ANY ($1))
