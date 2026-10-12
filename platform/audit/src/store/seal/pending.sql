-- Whether a committed record follows the sealed position.
SELECT EXISTS (
    SELECT
    FROM audit.sealer AS s
    JOIN audit.record AS r ON (r.txid, r.sequence) > (s.txid, s.sequence)
) AS "pending!"
