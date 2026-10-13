//! RFC 8785 canonicalization, RFC 9162 Merkle trees and Ed25519 block signatures.
use base64::Engine;
use chrono::{DateTime, TimeDelta, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use veoveo_audit_contract::*;
use veoveo_types::Sha256Digest;

const HEAD_DOMAIN: &[u8] = b"https://veoveo.ai/formats/audit-block\0";
const MAX_EXACT_JSON_INTEGER: u64 = (1u64 << 53) - 1;
const MAX_BLOCK_RECORDS: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum IntegrityError {
    #[error("audit signing key must be a base64-encoded 32-byte Ed25519 seed")]
    KeyFormat,
    #[error("audit JSON cannot be canonicalized")]
    Canonicalization,
    #[error("audit integer exceeds the I-JSON exact integer range")]
    IntegerRange,
    #[error("audit block membership or positions are invalid")]
    Membership,
    #[error("audit block chain has a missing or mismatched link")]
    Chain,
    #[error("audit record hash does not match the sealed Merkle root")]
    RecordHash,
    #[error("audit block hash does not match its head")]
    HeadHash,
    #[error("audit signing key is not in the trusted installation key ring")]
    UnknownKey,
    #[error("audit block signature is invalid")]
    Signature,
    #[error("audit sequence is exhausted")]
    Sequence,
    #[error("audit verification did not reach the required checkpoint")]
    Incomplete,
}
fn check_numbers(value: &serde_json::Value) -> Result<(), IntegrityError> {
    match value {
        serde_json::Value::Number(n)
            if n.as_u64().is_some_and(|n| n > MAX_EXACT_JSON_INTEGER)
                || n.as_i64()
                    .is_some_and(|n| n.unsigned_abs() > MAX_EXACT_JSON_INTEGER) =>
        {
            Err(IntegrityError::IntegerRange)
        }
        serde_json::Value::Array(values) => {
            for value in values {
                check_numbers(value)?;
            }
            Ok(())
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                check_numbers(value)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
pub fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, IntegrityError> {
    let json = serde_json::to_value(value).map_err(|_| IntegrityError::Canonicalization)?;
    check_numbers(&json)?;
    serde_json_canonicalizer::to_vec(&json).map_err(|_| IntegrityError::Canonicalization)
}
fn hash(parts: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part);
    }
    digest.finalize().into()
}
fn merkle(leaves: &[[u8; 32]]) -> [u8; 32] {
    match leaves.len() {
        0 => hash(&[b""]),
        1 => leaves[0],
        count => {
            let split = 1usize << ((usize::BITS - 1) - (count - 1).leading_zeros());
            hash(&[&[1], &merkle(&leaves[..split]), &merkle(&leaves[split..])])
        }
    }
}
/// The Merkle root over records' canonical bytes. A decoded record and the stored
/// record it came from have the same bytes, so either yields the sealed root.
pub fn record_root<R: AuditSealedRecord>(records: &[R]) -> Result<Sha256Digest, IntegrityError> {
    let leaves = records
        .iter()
        .map(|record| canonical_bytes(record).map(|bytes| hash(&[&[0], &bytes])))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Sha256Digest::from_bytes(merkle(&leaves)))
}
fn signed_head(head: &AuditBlockHead) -> Result<Vec<u8>, IntegrityError> {
    let mut bytes = HEAD_DOMAIN.to_vec();
    bytes.extend(canonical_bytes(head)?);
    Ok(bytes)
}
/// Holds a dedicated audit seed. Debug never includes key material.
pub struct AuditSigningKey(SigningKey);
impl std::fmt::Debug for AuditSigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditSigningKey")
            .field("key_id", &self.key_id())
            .finish()
    }
}
impl AuditSigningKey {
    pub fn from_base64(encoded: &str) -> Result<Self, IntegrityError> {
        let bytes = zeroize::Zeroizing::new(
            base64::engine::general_purpose::STANDARD
                .decode(encoded.trim())
                .map_err(|_| IntegrityError::KeyFormat)?,
        );
        if bytes.len() != 32 {
            return Err(IntegrityError::KeyFormat);
        }
        let mut seed = zeroize::Zeroizing::new([0u8; 32]);
        seed.copy_from_slice(&bytes);
        Ok(Self::from_seed(&seed))
    }
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self(SigningKey::from_bytes(seed))
    }
    pub fn public_key(&self) -> [u8; 32] {
        self.0.verifying_key().to_bytes()
    }
    pub fn key_id(&self) -> Sha256Digest {
        Sha256Digest::from_bytes(hash(&[&self.public_key()]))
    }
    /// Sign `records`, in commit order, as the block after `previous`. Each record takes
    /// the partition position after the one before it.
    pub fn seal<R: AuditSealedRecord>(
        &self,
        partition: AuditPartition,
        previous: Option<&AuditCheckpoint>,
        records: &[R],
        sealed_at: DateTime<Utc>,
    ) -> Result<AuditBlock, IntegrityError> {
        if previous.is_some_and(|previous| previous.partition != partition) {
            return Err(IntegrityError::Chain);
        }
        let sequence = previous
            .map_or(Some(1), |previous| previous.sequence.get().checked_add(1))
            .ok_or(IntegrityError::Sequence)?;
        let sequence = AuditBlockSequence::new(sequence).map_err(|_| IntegrityError::Sequence)?;
        let first = first_position(previous)?;
        let mut members = Vec::with_capacity(records.len());
        let mut position = first;
        for (index, record) in records.iter().enumerate() {
            if index > 0 {
                position = position.next().map_err(|_| IntegrityError::Sequence)?;
            }
            members.push(AuditBlockMember {
                id: record.record_id(),
                position,
            });
        }
        let head = AuditBlockHead {
            format: AuditBlockFormat::AuditBlock,
            partition,
            sequence,
            first_position: first,
            last_position: position,
            members,
            root: record_root(records)?,
            previous: previous.map(|previous| previous.head_hash.clone()),
            key_id: self.key_id(),
            sealed_at,
        };
        check_members(&head, records)?;
        let bytes = signed_head(&head)?;
        Ok(AuditBlock {
            head,
            head_hash: Sha256Digest::from_bytes(hash(&[&bytes])),
            signature: AuditSignature::from_bytes(self.0.sign(&bytes).to_bytes()),
        })
    }
}
/// The position of the first record after `previous`.
fn first_position(previous: Option<&AuditCheckpoint>) -> Result<AuditPosition, IntegrityError> {
    match previous {
        None => AuditPosition::new(1),
        Some(previous) => previous.last_position.next(),
    }
    .map_err(|_| IntegrityError::Sequence)
}
fn check_members<R: AuditSealedRecord>(
    head: &AuditBlockHead,
    records: &[R],
) -> Result<(), IntegrityError> {
    if records.is_empty()
        || records.len() > MAX_BLOCK_RECORDS
        || records.len() != head.members.len()
        || head.members.first().map(|m| m.position) != Some(head.first_position)
        || head.members.last().map(|m| m.position) != Some(head.last_position)
        || head
            .members
            .windows(2)
            .any(|pair| pair[0].position.next().ok() != Some(pair[1].position))
    {
        return Err(IntegrityError::Membership);
    }
    let mut seen = BTreeSet::new();
    for (member, record) in head.members.iter().zip(records) {
        if member.id != record.record_id()
            || record.record_partition() != &head.partition
            || !seen.insert(member.id)
        {
            return Err(IntegrityError::Membership);
        }
    }
    Ok(())
}
#[derive(Default)]
pub struct AuditKeyRing(BTreeMap<Sha256Digest, VerifyingKey>);
impl AuditKeyRing {
    pub fn insert(&mut self, key: [u8; 32]) -> Result<Sha256Digest, IntegrityError> {
        let id = Sha256Digest::from_bytes(hash(&[&key]));
        let public = VerifyingKey::from_bytes(&key).map_err(|_| IntegrityError::UnknownKey)?;
        if public.is_weak() {
            return Err(IntegrityError::UnknownKey);
        }
        self.0.insert(id.clone(), public);
        Ok(id)
    }
}
/// A record whose producer-supplied times disagree with the database clock that
/// recorded it by more than the verifier's allowed skew.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditFinding {
    BackdatedRecord {
        id: AuditRecordId,
        recorded_at: DateTime<Utc>,
    },
    FutureRecord {
        id: AuditRecordId,
        recorded_at: DateTime<Utc>,
    },
}
/// A verifier consumes one partition in order. A retained prefix requires its
/// externally trusted checkpoint; callers must finish against the expected tail.
pub struct AuditVerifier<'a> {
    keys: &'a AuditKeyRing,
    partition: AuditPartition,
    previous: Option<AuditCheckpoint>,
    clock_skew: TimeDelta,
}
impl<'a> AuditVerifier<'a> {
    pub fn new(
        keys: &'a AuditKeyRing,
        partition: AuditPartition,
        anchor: Option<AuditCheckpoint>,
        clock_skew: TimeDelta,
    ) -> Result<Self, IntegrityError> {
        if anchor.as_ref().is_some_and(|a| a.partition != partition)
            || clock_skew < TimeDelta::zero()
        {
            return Err(IntegrityError::Chain);
        }
        Ok(Self {
            keys,
            partition,
            previous: anchor,
            clock_skew,
        })
    }
    pub fn verify<R: AuditSealedRecord>(
        &mut self,
        block: &AuditBlock,
        records: &[R],
    ) -> Result<Vec<AuditFinding>, IntegrityError> {
        let head = &block.head;
        let expected_sequence = self
            .previous
            .as_ref()
            .map_or(Some(1), |p| p.sequence.get().checked_add(1))
            .ok_or(IntegrityError::Sequence)?;
        if head.partition != self.partition
            || head.sequence.get() != expected_sequence
            || head.previous != self.previous.as_ref().map(|p| p.head_hash.clone())
            || head.first_position != first_position(self.previous.as_ref())?
        {
            return Err(IntegrityError::Chain);
        }
        check_members(head, records)?;
        if record_root(records)? != head.root {
            return Err(IntegrityError::RecordHash);
        }
        let bytes = signed_head(head)?;
        if Sha256Digest::from_bytes(hash(&[&bytes])) != block.head_hash {
            return Err(IntegrityError::HeadHash);
        }
        let key = self
            .keys
            .0
            .get(&head.key_id)
            .ok_or(IntegrityError::UnknownKey)?;
        key.verify_strict(&bytes, &Signature::from_bytes(&block.signature.to_bytes()))
            .map_err(|_| IntegrityError::Signature)?;
        let mut findings = Vec::new();
        for record in records {
            // The database assigned `recorded_at` when the record was inserted, so a
            // sealer that runs hours later does not make a record look backdated.
            let recorded_at = record.record_recorded_at();
            let uuid_time = record
                .record_id()
                .as_uuid()
                .get_timestamp()
                .ok_or(IntegrityError::Membership)?
                .to_unix();
            let id_time = DateTime::from_timestamp(uuid_time.0 as i64, uuid_time.1)
                .ok_or(IntegrityError::Membership)?;
            let earliest = id_time.min(record.record_occurred_at());
            let latest = id_time.max(record.record_occurred_at());
            if recorded_at - earliest > self.clock_skew {
                findings.push(AuditFinding::BackdatedRecord {
                    id: record.record_id(),
                    recorded_at,
                });
            }
            if latest - recorded_at > self.clock_skew {
                findings.push(AuditFinding::FutureRecord {
                    id: record.record_id(),
                    recorded_at,
                });
            }
        }
        self.previous = Some(block.checkpoint());
        Ok(findings)
    }
    pub fn finish(self, expected: &AuditCheckpoint) -> Result<(), IntegrityError> {
        if self.previous.as_ref() != Some(expected) {
            return Err(IntegrityError::Incomplete);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> AuditRecord {
        AuditRecord {
            draft: AuditDraft::builder(
                AuditRequest::background(),
                veoveo_audit_contract::AuditRegistry::platform()
                    .detail(&veoveo_audit_contract::platform::AuditView {
                        partition: veoveo_audit_contract::AuditPartition::Installation,
                    })
                    .unwrap(),
                AuditOutcome::Success,
            )
            .build()
            .unwrap(),
            recorded_at: Utc::now(),
        }
    }
    #[test]
    fn rfc9162_uses_domain_separation_and_unbalanced_tree_split() {
        let a = hash(&[&[0], b"a"]);
        let b = hash(&[&[0], b"b"]);
        let c = hash(&[&[0], b"c"]);
        assert_eq!(merkle(&[]), hash(&[b""]));
        assert_eq!(merkle(&[a]), a);
        assert_eq!(
            merkle(&[a, b, c]),
            hash(&[&[1], &hash(&[&[1], &a, &b]), &c])
        );
        assert_ne!(merkle(&[a, b, c]), merkle(&[a, b, c, c]));
    }
    #[test]
    fn verification_detects_record_mutation_missing_links_and_forged_signatures() {
        let key = AuditSigningKey::from_seed(&[7; 32]);
        let mut keys = AuditKeyRing::default();
        keys.insert(key.public_key()).unwrap();
        let records = vec![fixture(), fixture(), fixture()];
        let block = key
            .seal(AuditPartition::Installation, None, &records, Utc::now())
            .unwrap();
        assert_eq!(
            block
                .head
                .members
                .iter()
                .map(|member| member.position.get())
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        let verify = |block: &AuditBlock, records: &[AuditRecord]| {
            AuditVerifier::new(
                &keys,
                AuditPartition::Installation,
                None,
                TimeDelta::minutes(5),
            )
            .unwrap()
            .verify(block, records)
        };
        assert!(verify(&block, &records).unwrap().is_empty());
        let mut changed = records.clone();
        changed[0].recorded_at -= TimeDelta::seconds(1);
        assert!(matches!(
            verify(&block, &changed),
            Err(IntegrityError::RecordHash)
        ));
        assert!(matches!(
            verify(&block, &records[..2]),
            Err(IntegrityError::Membership)
        ));
        let mut forged = block.clone();
        forged.signature = AuditSignature::from_bytes([0; 64]);
        assert!(matches!(
            verify(&forged, &records),
            Err(IntegrityError::Signature)
        ));
        let mut gap = block.clone();
        gap.head.members[1].position = AuditPosition::new(3).unwrap();
        assert!(matches!(
            verify(&gap, &records),
            Err(IntegrityError::Membership)
        ));
        let second_records = vec![fixture()];
        let second = key
            .seal(
                AuditPartition::Installation,
                Some(&block.checkpoint()),
                &second_records,
                Utc::now(),
            )
            .unwrap();
        assert_eq!(second.head.first_position.get(), 4);
        assert!(matches!(
            verify(&second, &second_records),
            Err(IntegrityError::Chain)
        ));
        let mut verifier = AuditVerifier::new(
            &keys,
            AuditPartition::Installation,
            None,
            TimeDelta::minutes(5),
        )
        .unwrap();
        verifier.verify(&block, &records).unwrap();
        assert!(matches!(
            verifier.finish(&second.checkpoint()),
            Err(IntegrityError::Incomplete)
        ));
    }
    #[test]
    fn late_commits_are_sealed_in_commit_order_and_backdating_is_reported() {
        let key = AuditSigningKey::from_seed(&[9; 32]);
        let mut keys = AuditKeyRing::default();
        keys.insert(key.public_key()).unwrap();
        let mut old = fixture();
        old.draft = AuditDraft::builder(
            old.draft.request().clone(),
            veoveo_audit_contract::AuditRegistry::platform()
                .detail(&veoveo_audit_contract::platform::TaskUpdate {})
                .unwrap(),
            AuditOutcome::Success,
        )
        .identity(old.draft.id())
        .occurred_at(Utc::now() - TimeDelta::hours(1))
        .build()
        .unwrap();
        let records = vec![fixture(), old];
        let block = key
            .seal(AuditPartition::Installation, None, &records, Utc::now())
            .unwrap();
        let mut verifier = AuditVerifier::new(
            &keys,
            AuditPartition::Installation,
            None,
            TimeDelta::minutes(5),
        )
        .unwrap();
        let findings = verifier.verify(&block, &records).unwrap();
        assert!(matches!(
            findings.as_slice(),
            [AuditFinding::BackdatedRecord { .. }]
        ));
        verifier.finish(&block.checkpoint()).unwrap();
    }
    #[test]
    fn database_clock_distinguishes_delayed_sealing_from_forged_record_times() {
        let key = AuditSigningKey::from_seed(&[17; 32]);
        let mut keys = AuditKeyRing::default();
        keys.insert(key.public_key()).unwrap();
        for offset in [0, -1, 1] {
            let mut record = fixture();
            // The producer supplies the occurrence time; the database supplies the
            // recording time. The sealer may legitimately be offline for two hours.
            let now = record.recorded_at;
            let supplied = now + TimeDelta::hours(offset);
            record.draft = AuditDraft::builder(
                record.draft.request().clone(),
                veoveo_audit_contract::AuditRegistry::platform()
                    .detail(&veoveo_audit_contract::platform::AuditView {
                        partition: veoveo_audit_contract::AuditPartition::Installation,
                    })
                    .unwrap(),
                AuditOutcome::Success,
            )
            .identity(record.draft.id())
            .occurred_at(supplied)
            .build()
            .unwrap();
            let block = key
                .seal(
                    AuditPartition::Installation,
                    None,
                    std::slice::from_ref(&record),
                    now + TimeDelta::hours(2),
                )
                .unwrap();
            let findings = AuditVerifier::new(
                &keys,
                AuditPartition::Installation,
                None,
                TimeDelta::minutes(5),
            )
            .unwrap()
            .verify(&block, &[record])
            .unwrap();
            match offset {
                0 => assert!(findings.is_empty()),
                -1 => assert!(matches!(
                    findings.as_slice(),
                    [AuditFinding::BackdatedRecord { .. }]
                )),
                1 => assert!(matches!(
                    findings.as_slice(),
                    [AuditFinding::FutureRecord { .. }]
                )),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn canonicalization_never_rounds_large_positions() {
        let stamp = AuditPosition::new(117_300_000_000_000_001).unwrap();
        assert_eq!(canonical_bytes(&stamp).unwrap(), br#""117300000000000001""#);
        assert!(matches!(
            canonical_bytes(&117_300_000_000_000_001u64),
            Err(IntegrityError::IntegerRange)
        ));
    }
}
