# Durable Task Runtime Instructions

Follow root AGENTS.md and DESIGN.md. Keep provider-specific state and effects in the
owning domain. Recovery must preserve uncertainty and current cancellation intent: an
`interrupted_indeterminate` Task is never run again.

Treat the `tasks` schema and the recovery classes as deployment contracts. Every worker
write compares the owner instance, and every caller read selects the caller in SQL
before decoding. Test claims, recovery, notifications and row-level security against
the PostgreSQL fixture.
