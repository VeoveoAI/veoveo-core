-- Give the listed Tasks the earliest creation time among them, to probe the page
-- keyset's ID order between Tasks created at one time. The caller turns the revision
-- trigger, which keeps a Task's creation time, off around it in one transaction.
UPDATE tasks.task
SET created_at = (SELECT min(created_at) FROM tasks.task WHERE id = ANY ($1::uuid[]))
WHERE id = ANY ($1::uuid[])
