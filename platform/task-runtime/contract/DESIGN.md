# Task Contract

## Standards And Protocols

| Boundary | Supported profile |
|---|---|
| JSON and JSON Schema 2020-12 | Closed Task status and recovery-class strings through Serde and the shared Vocabulary declaration |
| PostgreSQL 18 through sqlx 0.9.0 | With the `postgres` feature, both vocabularies bind to `text` columns using their wire spellings |

## Ownership And Dependencies

This crate owns `TaskStatus` and `RecoveryClass`. Task Runtime applies recovery and
transitions, and Console describes them to the browser. Status declaration order and
the seven wire spellings are stable. Recovery declares `resume` and
`interrupted_indeterminate` in that order. A `resume` Task is claimed and run again
after its owning process ends; an `interrupted_indeterminate` Task fails as interrupted,
because its effect may already have happened.

The default dependency graph contains foundational types, serialization and schema
support. The `postgres` feature adds sqlx and the Vocabulary `postgres` hook, which
encodes the wire spelling and rejects any other stored value on decode. Storage crates
select it; contract consumers acquire no database driver by default. A separate
package lets storage and Task Runtime reuse the declaration without a dependency
cycle.

Task request, owner, failure and contribution envelopes have their existing owners.
This crate does not enumerate operation types or domain result payloads. Tests cover
complete vocabularies, unknown-string rejection and the PostgreSQL type mapping.
