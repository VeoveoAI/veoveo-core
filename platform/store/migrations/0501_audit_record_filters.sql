-- A page that filters one partition by actor, trace or occurrence time seeks one of these
-- indexes instead of reading the partition in identity order. Each ends in the record
-- identity, the order a page reads in.
CREATE INDEX record_actor ON audit.record (partition, actor, id) WHERE actor IS NOT NULL;
CREATE INDEX record_trace ON audit.record (partition, trace_id, id);
CREATE INDEX record_occurred ON audit.record (partition, occurred_at, id);
