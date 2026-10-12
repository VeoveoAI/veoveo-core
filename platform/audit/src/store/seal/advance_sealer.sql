-- Move the sealed position from the one the sealer read to the last record it sealed.
UPDATE audit.sealer AS s
SET txid = $1::bigint::text::xid8, sequence = $2
WHERE s.txid = $3::bigint::text::xid8 AND s.sequence = $4
