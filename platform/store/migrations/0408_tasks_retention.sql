-- Every table states how long its rows stay and what removes them.
COMMENT ON TABLE tasks.task IS
    'Retention: until retention_expires_at passes for a finished Task that no consumer '
    'pins; its server''s Task runtime prunes such Tasks in batches.';

COMMENT ON TABLE tasks.task_input IS
    'Retention: as long as its Task.';
