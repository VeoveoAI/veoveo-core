-- veoveo: all tenants (the sealer seals the records of every partition in commit order)
-- Whether a committed record follows the sealed position. The position is read once, so
-- the comparison seeks record_commit_order past it.
SELECT EXISTS (
    SELECT
    FROM audit.record AS r
    WHERE (r.txid, r.sequence) > (SELECT s.txid, s.sequence FROM audit.sealer AS s)
) AS "pending!"
