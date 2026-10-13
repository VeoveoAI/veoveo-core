-- Every table states how long its rows stay and what removes them.
COMMENT ON TABLE kernel.instance IS
    'Retention: while its process runs; a clean shutdown deletes the row, and '
    'kernel.reap_instances() deletes it once its lock is free.';
