-- Every table states how long its rows stay and what removes them.
COMMENT ON TABLE audit.record IS
    'Retention: until retention retires the sealed block that holds it, once every '
    'configured destination acknowledged the block.';

COMMENT ON TABLE audit.sealer IS
    'Retention: one row for the life of the installation.';

COMMENT ON TABLE audit.partition_head IS
    'Retention: one row per partition for the life of the installation.';

COMMENT ON TABLE audit.block IS
    'Retention: until audit.retire_block() retires it, once it was sealed before the '
    'cutoff and every configured destination acknowledged it.';

COMMENT ON TABLE audit.retention_anchor IS
    'Retention: one row per partition for the life of the installation; each retired '
    'block replaces it.';

COMMENT ON TABLE audit.export_delivery IS
    'Retention: as long as its block.';

COMMENT ON TABLE audit.export_cursor IS
    'Retention: one row per destination and partition for the life of the installation.';

COMMENT ON TABLE audit.indexing_receipt IS
    'Retention: one day; the indexing writer prunes older receipts in batches.';

COMMENT ON TABLE audit.indexing_window IS
    'Retention: until the window ends and the finalizer replaces it with one audit '
    'record.';
