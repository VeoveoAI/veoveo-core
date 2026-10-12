-- Work Contexts the new revision no longer defines. Their grants, membership rules and
-- Task routes go with them.
DELETE FROM identity.work_context
WHERE NOT (id = ANY ($1))
