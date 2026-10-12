-- Whether the record is in the partition and at or below the sealed commit position.
SELECT EXISTS (
    SELECT
    FROM audit.record AS r, audit.sealer AS s
    WHERE r.id = $1
        AND r.partition = $2
        AND (r.txid, r.sequence) <= (s.txid, s.sequence)
) AS "sealed!"
