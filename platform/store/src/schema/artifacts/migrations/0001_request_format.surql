-- Current-only request receipt admission. No default, backfill or conversion.
DEFINE FIELD request_format ON artifact_write_redemption TYPE 'https://veoveo.ai/formats/artifact-write-request' PERMISSIONS FULL;
