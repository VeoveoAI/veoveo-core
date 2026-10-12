-- Row counts of the audit tables that export and retention change.
SELECT
    (SELECT count(*) FROM audit.record) AS "records!",
    (SELECT count(*) FROM audit.block) AS "blocks!",
    (SELECT count(*) FROM audit.export_delivery) AS "deliveries!",
    (SELECT count(*) FROM audit.export_cursor) AS "cursors!",
    (SELECT count(*) FROM audit.retention_anchor) AS "anchors!"
