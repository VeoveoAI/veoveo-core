# Task Contract

## Standards And Protocols

| Boundary | Supported profile |
|---|---|
| JSON and JSON Schema 2020-12 | Closed Task status and recovery-class strings through Serde and the shared Vocabulary declaration |

## Ownership And Dependencies

This crate owns `TaskStatus` and `RecoveryClass`. Task Runtime applies recovery and
transitions, and Console describes them to the browser. Status declaration order and
the seven wire spellings are stable. Recovery declares `resume`, `webhook_wait`,
`provider_wait` and `interrupted_indeterminate` in that order. The execution service
owns the behavior of each recovery class.

The dependency graph contains foundational types, serialization and schema support. A
separate package lets storage and Task Runtime reuse the declaration without a
dependency cycle.

Task request, owner, failure and contribution envelopes have their existing owners.
This crate does not enumerate operation types or domain result payloads. Pure tests
cover complete vocabularies and unknown-string rejection.
