# Audit Log

Follow root AGENTS.md, docs/DATABASE.md and this crate's DESIGN.md. The contract crate
owns wire types and the transactional append; this crate owns the `audit` schema's
queries, batching, signing, sealing, export and verification. Preserve each caller's
commit acknowledgement. Never move required audit writes behind response delivery.
Domain writes append inside their existing transaction. Sealer writes run on the
connection that holds the `audit.sealer` role. Keep signing keys redacted and separate
from gateway assertion keys.
