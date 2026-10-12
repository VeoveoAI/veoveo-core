-- The commit position up to which every record is sealed.
SELECT s.txid::text::bigint AS "txid!", s.sequence
FROM audit.sealer AS s
