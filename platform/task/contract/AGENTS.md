# Task Contract Instructions

Follow root instructions and this component's DESIGN.md. Keep lifecycle and recovery
values independent of execution services and storage. The `postgres` feature owns the
database binding; default consumers must not enable it. Preserve spelling and
declaration order. Recovery semantics and domain payloads stay with their existing
owners.
