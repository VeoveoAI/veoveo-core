# Audit Export

## Standards And Protocols

| Boundary | Profile |
|---|---|
| [OCSF 1.9.0](https://github.com/ocsf/ocsf-schema/tree/v1.9.0) | Base Event and API Activity mapping of the record envelope; JSON Lines, one event per source record |
| [S3 PutObject](https://docs.aws.amazon.com/AmazonS3/latest/API/API_PutObject.html) and [GetObject](https://docs.aws.amazon.com/AmazonS3/latest/API/API_GetObject.html) | SigV4, conditional object creation, SHA-256 request checksum and downloaded-byte reconciliation; optional Object Lock compliance mode |
| [OTLP/HTTP](https://opentelemetry.io/docs/specs/otlp/) | JSON Logs requests, string-encoded nanosecond timestamps, hexadecimal trace/span IDs and explicit collector acknowledgement |
| `veoveo.audit.export/v1` | Destination identity includes the wire mapping version and public destination configuration |
| RFC 8785, RFC 9162 and RFC 8032 | The companion JSON object contains the canonical signed audit block; its member records are preserved in each OCSF event |

## Configuration And Ownership

The component that runs the audit service, the Hub in an installation, loads at
most 64 KiB from `VEOVEO_AUDIT_EXPORT_CONFIG`. The closed JSON
configuration has optional `s3` and `otlp` destinations. Audit owns their types and
their delivery state in the `audit` schema. S3 bucket names use lowercase letters, digits
and hyphens. Prefixes contain checked path segments. URL builders encode individual
segments, and endpoint validation rejects embedded credentials, queries and fragments.
HTTPS is required unless the installation explicitly selects `allow_http`.

S3 uses the existing `object_store` credential provider and signer. OTLP optionally
reads a bearer token from the configured environment variable. Neither credential
value enters destination identity, audit content, object keys or diagnostics. HTTP
redirects are disabled. A changed destination, prefix, Object Lock policy or mapping
version produces a new identity and requires delivery of every retained block to it.
A credential rotation preserves receipts for the same destination.

Export configuration changes require a coordinated drain and restart of that component. They
apply to retained blocks; already retired blocks must be copied from their archive
by the installation when moving to another destination.

## Delivery And Recovery

The replica that holds the `audit.sealer` role runs the exporter as its own task. The
exporter listens on the `audit` channel before replaying pending blocks and wakes when a
block is sealed or the listener reconnects; an idle exporter performs no periodic
database scan. Each candidate query compares partition heads with delivery cursors and
retention anchors in SQL before its one-block limit. It reads the selected block by
partition and sequence and rotates partitions by last delivery time; it never scans
already exported block history. No request handler waits for export.

Before sending data, the exporter checks the block's record root and commits its
destination and both payload hashes through `audit.record_export`. The intent, the
receipt and a refusal are written on the connection that holds the sealer role, so a
replica that lost the role cannot record them. Repeating an intent with different hashes
fails. A receipt marks the intent delivered and advances the destination's partition
cursor in one transaction. Retention requires a receipt from every configured
destination and removes per-block intents with the retired block. The last retired block
and each delivery cursor survive the deletion.

S3 stores an OCSF JSON Lines object and a signed-block JSON object. Keys include the
destination identity, a partition digest, the block sequence and its hash. The
exporter first reads the key and compares bytes. An absent key permits a PUT with
`If-None-Match: *`; a lost response leads to another GET of that key. Both objects
must match before the exporter records delivery. No retry overwrites a different object.

Compliance mode adds retention headers and verifies the returned mode, retention
date and non-null version ID on GET. Its date is the block's sealing time plus the
configured days, making retries stable. An already expired export window requires
a longer configured period. A provider that ignores Object Lock cannot satisfy
this profile. The bundled SeaweedFS destination uses `disabled`; it establishes no
write-once storage guarantee.

OTLP sends one request per block with events named `veoveo.audit.<class>`. Its body
contains the OCSF event, and attributes preserve record, partition and request IDs.
The exporter sends HTTP directly because a stored receipt must reflect collector
acknowledgement. HTTP 200 with zero rejected logs acknowledges delivery. Partial
rejection is persisted and stops automatic replay of that intent, including on
another replica. The installation must inspect the rejected destination and select
a corrected destination configuration; changing credentials alone does not erase
a recorded rejection. Partial acceptance never permits retention.

Network failures and uncertain acknowledgements keep their intent for retry after
five seconds. OTLP honors `Retry-After` on retryable statuses. Delivery is at least
once: a crash after collector acceptance but before the receipt commits can repeat
records. Consumers deduplicate by record ID. Rejection persistence also has this
crash window; an uncommitted response cannot establish what another replica observed.

Each HTTP operation has a five-second connection deadline and a fifteen-second total
deadline. A block network attempt has sixty seconds. A known rejection stays in memory
while its database write retries every five seconds; a network deadline cannot turn that
response into a resend. Content is limited to 32 MiB, the signed head to 2 MiB, and
collector responses to 64 KiB. Provider bodies and error text never enter logs.
Shutdown cancels export after stopping producers and sealing committed records.
Unacknowledged intents resume on the replica that next holds the sealer role; shutdown
does not wait for an unavailable archive to recover.

## OCSF Mapping

`cargo test -p veoveo-audit --test ocsf -- --ignored --nocapture` submits synthetic
fixtures for each mapped event shape and outcome to the upstream
[OCSF 1.9.0 validation API](https://schema.ocsf.io/1.9.0/doc/swagger.json).
The harness checks the reported schema version, limits each request to 15
seconds and the run to 180 seconds, and prints structured validation results.
It reads no installation records. This network qualification complements the local
mapping tests; default test runs do not contact the public validator.

The exporter reads each block's members as stored records, so it needs no owner's
detail declaration. Every event preserves the complete stored record as
`unmapped.veoveo`, including attribution, reason and timestamps. This supplies the
record bytes for verification against the signed block.

| Record | OCSF 1.9.0 |
|---|---|
| Outcome `success`, `failure`, `denied`, `unknown` | `status_id` 1, 2, 2, 0, and the Security Control profile's `action_id` 1 (Allowed), 1, 2 (Denied), 0 |
| Reason | `status_code` |
| Acting server and its Task | `actor.application` name and uid |
| Target | `resources[].uid` |
| Tenant, request | `metadata.tenant_uid`, `metadata.correlation_uid` |

A record with an actor and a source endpoint maps to API Activity 6003 with activity
Other; its activity name and API operation are the record's class, such as
`task.update`. Each class also declares the OCSF class and activity it maps to; the
exporter adopts those declarations in a later step. Any other record maps to Base Event and preserves the source without
inventing an actor, username or address. The mapping reads only the envelope, so it
names no owner, and each owner's detail reaches the archive in `unmapped.veoveo`.

## Qualification

`cargo test --locked --workspace --all-features --test s3_export -- --ignored --nocapture`
qualifies actual S3 delivery with the hosted writer and sealer against a disposable
database from the PostgreSQL fixture. `VEOVEO_AUDIT_TEST_S3_CONFIG` names a public
export configuration with an S3 destination, an existing bucket and Object Lock
disabled. The normal S3 credential chain supplies credentials, and the PostgreSQL 18
programs must be on `PATH`. The fixture adds a unique prefix, compares downloaded source
records and signed blocks, checks receipts from a second pool, and restarts the audit
service. It
allows 180 seconds for acceptance and 30 seconds for prefix cleanup, including after
a failed assertion. The configured S3 provider is externally managed. This test can
use the bundled SeaweedFS endpoint; it does not qualify a deployment or Object Lock. Default test runs leave this provider test ignored.

The fixture also accepts an OTLP destination in that configuration. It waits for both
destinations' receipts before restarting the worker and checks both delivery cursors
after restart. The collector operator must verify that its logs pipeline received the
two synthetic records; an acknowledgement alone proves receiver acceptance.

The unit fixtures cover uncertain S3 PUTs, mismatched bytes, missing Object Lock proof,
destination changes and OTLP partial rejection. `tests/export.rs` covers immutable
intents, idempotent receipts, refusals that bind every replica, retention that waits
for every destination, and one collector acknowledgement per record while sealing and
export overlap. Installed delivery and compliance-mode provider acceptance are qualified
outside this repository.

## Destination Text Admission

Bucket names contain 3–63 lowercase ASCII letters, digits or hyphens with alphanumeric
endpoints. Object prefixes contain 1–256 bytes of ASCII alphanumeric, hyphen,
underscore, dot and slash; every slash-separated segment is nonempty and differs
from `.` and `..`. Prefixes may contain uppercase letters. Owner validators preserve
input bytes and feed the nominal String conversion used by Serde.
