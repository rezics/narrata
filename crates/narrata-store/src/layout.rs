//! Key-space layout v1 of the save engine (ADR 0014): the spaces of the Stage 1–5 registrant.
//!
//! The history layer owns `meta`, `touch`, `refs`, `pins` and `children` and their codecs
//! (ADR 0015); this module adds the catalog, archive, Compound Save, input, Effect ledger,
//! fence, catalog operation, Program and per-turn commit spaces. Keys are raw fixed-width
//! identities and `RefName` bytes; values are canonical CBOR maps decoded strictly, because the
//! backend is outside the trust boundary.

use narrata_core::{
    CapabilityId, CapabilityVersion, CommitId, CompoundSaveManifestId, DeliveryPolicy,
    DiagnosticId, EffectId, EffectRequestDigest, ExecutionId, InputId, InputPayloadDigest,
    ObjectId, ProgramArtifactId, RewindPolicy, TimelineArchiveManifestId, TimelineCatalogEventId,
    codec::{CborReader, CborWriter as Writer, DecodeError},
};
pub use narrata_history::layout::{
    CHILDREN, GRAPH_KEY, LAYOUT_KEY, LAYOUT_VERSION, META, PINS, REFS, SWEEP_KEY, TOUCH,
};
use narrata_history::{
    HistoryError,
    layout::{
        decode, decode_single_id, decode_single_unsigned, expect_map, fixed, key, name, named,
        names, single_bytes, single_unsigned, split,
    },
};
use narrata_storage::KeySpace;

use crate::{
    CatalogRefKey, CompoundSaveRefKey, EffectLedgerEntry, LeaseId, LedgerFence, LedgerStatus,
    RefName, TimelineArchiveRefKey, TimelineCoverage, TimelineOperationId,
};

pub const CATALOG_HEADS: KeySpace = KeySpace::new(3);
pub const ARCHIVES: KeySpace = KeySpace::new(4);
pub const COMPOUND_SAVES: KeySpace = KeySpace::new(5);
pub const INPUTS: KeySpace = KeySpace::new(6);
pub const EFFECTS: KeySpace = KeySpace::new(8);
pub const LEDGER_FENCES: KeySpace = KeySpace::new(9);
pub const CATALOG_OPERATIONS: KeySpace = KeySpace::new(10);
pub const PROGRAMS: KeySpace = KeySpace::new(11);
pub const COMMITS: KeySpace = KeySpace::new(13);

/// Every space a Stage 1–5 store writes, with its name, in space order.
pub const SPACES: [(KeySpace, &str); 14] = [
    (META, "meta"),
    (TOUCH, "touch"),
    (REFS, "refs"),
    (CATALOG_HEADS, "catalog-heads"),
    (ARCHIVES, "archives"),
    (COMPOUND_SAVES, "compound-saves"),
    (INPUTS, "inputs"),
    (PINS, "pins"),
    (EFFECTS, "effects"),
    (LEDGER_FENCES, "ledger-fences"),
    (CATALOG_OPERATIONS, "catalog-operations"),
    (PROGRAMS, "programs"),
    (CHILDREN, "children"),
    (COMMITS, "commits"),
];

/// The spaces this registrant adds to the history layer's.
pub(crate) const OWN_SPACES: [KeySpace; 9] = [
    CATALOG_HEADS,
    ARCHIVES,
    COMPOUND_SAVES,
    INPUTS,
    EFFECTS,
    LEDGER_FENCES,
    CATALOG_OPERATIONS,
    PROGRAMS,
    COMMITS,
];

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

// catalog heads

pub(crate) fn catalog_key(key: &CatalogRefKey) -> Vec<u8> {
    key.timeline().as_bytes().to_vec()
}

pub(crate) fn decode_catalog_key(bytes: &[u8]) -> Result<CatalogRefKey, HistoryError> {
    fixed::<16>("Catalog Head key", bytes)
        .map(|bytes| CatalogRefKey::new(ExecutionId::from_bytes(bytes)))
}

pub(crate) fn encode_catalog_head(
    event: TimelineCatalogEventId,
    coverage: TimelineCoverage,
) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.map(2);
    writer.unsigned(0);
    writer.bytes(event.as_bytes());
    writer.unsigned(1);
    match coverage {
        TimelineCoverage::FromBaseline { baseline } => {
            writer.array(2);
            writer.unsigned(0);
            writer.bytes(baseline.as_bytes());
        }
        TimelineCoverage::Imported { baseline, source } => {
            writer.array(3);
            writer.unsigned(1);
            writer.bytes(baseline.as_bytes());
            writer.bytes(source.as_bytes());
        }
    }
    writer.into_bytes()
}

pub(crate) fn decode_catalog_head(
    bytes: &[u8],
) -> Result<(TimelineCatalogEventId, TimelineCoverage), HistoryError> {
    decode(
        "Catalog Head value",
        bytes,
        |reader| {
            expect_map(reader, 2)?;
            key(reader, 0)?;
            let event = TimelineCatalogEventId::from_bytes(reader.bytes_exact::<32>()?);
            key(reader, 1)?;
            let length = reader.array_len()?;
            let coverage = match (length, reader.unsigned()?) {
                (2, 0) => TimelineCoverage::FromBaseline {
                    baseline: CommitId::from_bytes(reader.bytes_exact::<32>()?),
                },
                (3, 1) => TimelineCoverage::Imported {
                    baseline: CommitId::from_bytes(reader.bytes_exact::<32>()?),
                    source: TimelineArchiveManifestId::from_bytes(reader.bytes_exact::<32>()?),
                },
                _ => return Err(DecodeError::Schema("Catalog coverage")),
            };
            Ok((event, coverage))
        },
        |(event, coverage)| encode_catalog_head(*event, *coverage),
    )
}

// archives

pub(crate) fn archive_key(key: &TimelineArchiveRefKey) -> Vec<u8> {
    concat(&[key.timeline().as_bytes(), key.name().as_str().as_bytes()])
}

pub(crate) fn decode_archive_key(bytes: &[u8]) -> Result<TimelineArchiveRefKey, HistoryError> {
    let (timeline, rest) = split("Archive key", bytes, 16)?;
    Ok(TimelineArchiveRefKey::new(
        ExecutionId::from_bytes(fixed("Archive key", timeline)?),
        name("Archive key", rest)?,
    ))
}

pub(crate) fn encode_archive(manifest: TimelineArchiveManifestId) -> Vec<u8> {
    single_bytes(manifest.as_bytes())
}

pub(crate) fn decode_archive(bytes: &[u8]) -> Result<TimelineArchiveManifestId, HistoryError> {
    decode_single_id("Archive value", bytes).map(TimelineArchiveManifestId::from_bytes)
}

// compound saves

pub(crate) fn compound_save_key(key: &CompoundSaveRefKey) -> Vec<u8> {
    named(
        key.owner().as_str().as_bytes(),
        key.slot().as_str().as_bytes(),
    )
}

pub(crate) fn compound_save_prefix(owner: Option<&RefName>) -> Vec<u8> {
    owner.map_or_else(Vec::new, |owner| named(owner.as_str().as_bytes(), &[]))
}

pub(crate) fn decode_compound_save_key(bytes: &[u8]) -> Result<CompoundSaveRefKey, HistoryError> {
    let (owner, slot) = names("Compound Save key", bytes)?;
    Ok(CompoundSaveRefKey::new(owner, slot))
}

pub(crate) fn encode_compound_save(manifest: CompoundSaveManifestId) -> Vec<u8> {
    single_bytes(manifest.as_bytes())
}

pub(crate) fn decode_compound_save(bytes: &[u8]) -> Result<CompoundSaveManifestId, HistoryError> {
    decode_single_id("Compound Save value", bytes).map(CompoundSaveManifestId::from_bytes)
}

// inputs

pub(crate) fn input_key(execution: ExecutionId, input: InputId) -> Vec<u8> {
    concat(&[execution.as_bytes(), input.as_bytes()])
}

pub(crate) fn encode_input(
    parent: CommitId,
    payload: InputPayloadDigest,
    commit: CommitId,
) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.map(3);
    writer.unsigned(0);
    writer.bytes(parent.as_bytes());
    writer.unsigned(1);
    writer.bytes(payload.as_bytes());
    writer.unsigned(2);
    writer.bytes(commit.as_bytes());
    writer.into_bytes()
}

pub(crate) fn decode_input(
    bytes: &[u8],
) -> Result<(CommitId, InputPayloadDigest, CommitId), HistoryError> {
    decode(
        "Input value",
        bytes,
        |reader| {
            expect_map(reader, 3)?;
            key(reader, 0)?;
            let parent = CommitId::from_bytes(reader.bytes_exact::<32>()?);
            key(reader, 1)?;
            let payload = InputPayloadDigest::from_bytes(reader.bytes_exact::<32>()?);
            key(reader, 2)?;
            let commit = CommitId::from_bytes(reader.bytes_exact::<32>()?);
            Ok((parent, payload, commit))
        },
        |(parent, payload, commit)| encode_input(*parent, *payload, *commit),
    )
}

// effects

pub(crate) fn effect_key(execution: ExecutionId, effect: EffectId) -> Vec<u8> {
    concat(&[execution.as_bytes(), effect.as_bytes()])
}

pub(crate) fn decode_effect_key(bytes: &[u8]) -> Result<(ExecutionId, EffectId), HistoryError> {
    let (execution, effect) = split("Effect key", bytes, 16)?;
    Ok((
        ExecutionId::from_bytes(fixed("Effect key", execution)?),
        EffectId::from_bytes(fixed("Effect key", effect)?),
    ))
}

const fn delivery_code(policy: DeliveryPolicy) -> u64 {
    match policy {
        DeliveryPolicy::Reconcile => 0,
        DeliveryPolicy::RecordedQuery => 1,
        DeliveryPolicy::AtLeastOnceIdempotent => 2,
        DeliveryPolicy::HostTransactional => 3,
    }
}

const fn delivery_from_code(code: u64) -> Option<DeliveryPolicy> {
    match code {
        0 => Some(DeliveryPolicy::Reconcile),
        1 => Some(DeliveryPolicy::RecordedQuery),
        2 => Some(DeliveryPolicy::AtLeastOnceIdempotent),
        3 => Some(DeliveryPolicy::HostTransactional),
        _ => None,
    }
}

pub(crate) fn encode_effect(entry: &EffectLedgerEntry) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.map(7);
    writer.unsigned(0);
    writer.bytes(entry.request_digest.as_bytes());
    writer.unsigned(1);
    writer.text(entry.capability.as_str());
    writer.unsigned(2);
    writer.unsigned(u64::from(entry.capability_version.get()));
    writer.unsigned(3);
    writer.bytes(entry.origin_commit.as_bytes());
    writer.unsigned(4);
    writer.unsigned(delivery_code(entry.delivery));
    writer.unsigned(5);
    match &entry.rewind {
        RewindPolicy::Reapply => {
            writer.array(1);
            writer.unsigned(0);
        }
        RewindPolicy::ReuseRecordedResponse => {
            writer.array(1);
            writer.unsigned(1);
        }
        RewindPolicy::Barrier => {
            writer.array(1);
            writer.unsigned(2);
        }
        RewindPolicy::Compensatable { capability } => {
            writer.array(2);
            writer.unsigned(3);
            writer.text(capability.as_str());
        }
    }
    writer.unsigned(6);
    match &entry.status {
        LedgerStatus::Claimed { lease, expires_at } => {
            writer.array(3);
            writer.unsigned(0);
            writer.bytes(lease.as_bytes());
            writer.unsigned(*expires_at);
        }
        LedgerStatus::Completed { response, fence } => {
            writer.array(3);
            writer.unsigned(1);
            writer.bytes(response.as_bytes());
            writer.unsigned(fence.get());
        }
        LedgerStatus::Rejected { response, fence } => {
            writer.array(3);
            writer.unsigned(2);
            writer.bytes(response.as_bytes());
            writer.unsigned(fence.get());
        }
        LedgerStatus::RetryableFailure { diagnostic } => {
            writer.array(2);
            writer.unsigned(3);
            writer.bytes(diagnostic.as_bytes());
        }
        LedgerStatus::UnknownOutcome { diagnostic, fence } => {
            writer.array(3);
            writer.unsigned(4);
            writer.bytes(diagnostic.as_bytes());
            writer.unsigned(fence.get());
        }
        LedgerStatus::Compensated {
            response,
            original_fence,
            by_effect,
            compensation_fence,
        } => {
            writer.array(5);
            writer.unsigned(5);
            writer.bytes(response.as_bytes());
            writer.unsigned(original_fence.get());
            writer.bytes(by_effect.as_bytes());
            writer.unsigned(compensation_fence.get());
        }
    }
    writer.into_bytes()
}

fn capability(reader: &mut CborReader<'_>) -> Result<CapabilityId, DecodeError> {
    CapabilityId::new(reader.text(64)?).map_err(|_| DecodeError::Schema("Effect capability"))
}

fn terminal_fence(reader: &mut CborReader<'_>) -> Result<LedgerFence, DecodeError> {
    match reader.unsigned()? {
        0 => Err(DecodeError::Schema("terminal ledger fence is zero")),
        value => Ok(LedgerFence::from_u64(value)),
    }
}

pub(crate) fn decode_effect(
    key_bytes: &[u8],
    bytes: &[u8],
) -> Result<EffectLedgerEntry, HistoryError> {
    let (execution, effect) = decode_effect_key(key_bytes)?;
    decode(
        "Effect value",
        bytes,
        |reader| {
            expect_map(reader, 7)?;
            key(reader, 0)?;
            let request_digest = EffectRequestDigest::from_bytes(reader.bytes_exact::<32>()?);
            key(reader, 1)?;
            let capability_id = capability(reader)?;
            key(reader, 2)?;
            let version =
                u16::try_from(reader.unsigned()?).map_err(|_| DecodeError::IntegerOverflow)?;
            let capability_version = CapabilityVersion::new(version)
                .ok_or(DecodeError::Schema("Effect capability version"))?;
            key(reader, 3)?;
            let origin_commit = CommitId::from_bytes(reader.bytes_exact::<32>()?);
            key(reader, 4)?;
            let delivery = delivery_from_code(reader.unsigned()?)
                .ok_or(DecodeError::Schema("Effect delivery policy"))?;
            key(reader, 5)?;
            let length = reader.array_len()?;
            let rewind = match (length, reader.unsigned()?) {
                (1, 0) => RewindPolicy::Reapply,
                (1, 1) => RewindPolicy::ReuseRecordedResponse,
                (1, 2) => RewindPolicy::Barrier,
                (2, 3) => RewindPolicy::Compensatable {
                    capability: capability(reader)?,
                },
                _ => return Err(DecodeError::Schema("Effect rewind policy")),
            };
            key(reader, 6)?;
            let length = reader.array_len()?;
            let status = match (length, reader.unsigned()?) {
                (3, 0) => LedgerStatus::Claimed {
                    lease: LeaseId::from_bytes(reader.bytes_exact::<16>()?),
                    expires_at: reader.unsigned()?,
                },
                (3, 1) => LedgerStatus::Completed {
                    response: ObjectId::from_bytes(reader.bytes_exact::<32>()?),
                    fence: terminal_fence(reader)?,
                },
                (3, 2) => LedgerStatus::Rejected {
                    response: ObjectId::from_bytes(reader.bytes_exact::<32>()?),
                    fence: terminal_fence(reader)?,
                },
                (2, 3) => LedgerStatus::RetryableFailure {
                    diagnostic: DiagnosticId::from_bytes(reader.bytes_exact::<32>()?),
                },
                (3, 4) => LedgerStatus::UnknownOutcome {
                    diagnostic: DiagnosticId::from_bytes(reader.bytes_exact::<32>()?),
                    fence: terminal_fence(reader)?,
                },
                (5, 5) => LedgerStatus::Compensated {
                    response: ObjectId::from_bytes(reader.bytes_exact::<32>()?),
                    original_fence: terminal_fence(reader)?,
                    by_effect: EffectId::from_bytes(reader.bytes_exact::<32>()?),
                    compensation_fence: terminal_fence(reader)?,
                },
                _ => return Err(DecodeError::Schema("Effect status")),
            };
            Ok(EffectLedgerEntry {
                execution,
                effect,
                request_digest,
                capability: capability_id,
                capability_version,
                origin_commit,
                delivery,
                rewind,
                status,
            })
        },
        encode_effect,
    )
}

// ledger fences

pub(crate) fn fence_key(execution: ExecutionId) -> Vec<u8> {
    execution.as_bytes().to_vec()
}

pub(crate) fn encode_fence(fence: LedgerFence) -> Vec<u8> {
    single_unsigned(fence.get())
}

pub(crate) fn decode_fence(bytes: &[u8]) -> Result<LedgerFence, HistoryError> {
    decode_single_unsigned("ledger fence value", bytes).map(LedgerFence::from_u64)
}

// catalog operations

pub(crate) fn catalog_operation_key(
    execution: ExecutionId,
    operation: TimelineOperationId,
) -> Vec<u8> {
    concat(&[execution.as_bytes(), operation.as_bytes()])
}

pub(crate) fn encode_catalog_operation(
    previous: Option<TimelineCatalogEventId>,
    event: TimelineCatalogEventId,
) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.map(2);
    writer.unsigned(0);
    match previous {
        Some(previous) => writer.bytes(previous.as_bytes()),
        None => writer.null(),
    }
    writer.unsigned(1);
    writer.bytes(event.as_bytes());
    writer.into_bytes()
}

pub(crate) fn decode_catalog_operation(
    bytes: &[u8],
) -> Result<(Option<TimelineCatalogEventId>, TimelineCatalogEventId), HistoryError> {
    decode(
        "Catalog operation value",
        bytes,
        |reader| {
            expect_map(reader, 2)?;
            key(reader, 0)?;
            let previous = reader
                .optional(|reader| reader.bytes_exact::<32>())?
                .map(TimelineCatalogEventId::from_bytes);
            key(reader, 1)?;
            let event = TimelineCatalogEventId::from_bytes(reader.bytes_exact::<32>()?);
            Ok((previous, event))
        },
        |(previous, event)| encode_catalog_operation(*previous, *event),
    )
}

// programs

pub(crate) fn program_key(artifact: ProgramArtifactId) -> Vec<u8> {
    artifact.as_bytes().to_vec()
}

pub(crate) fn encode_program(object: narrata_history::ObjectId) -> Vec<u8> {
    single_bytes(object.as_bytes())
}

pub(crate) fn decode_program(bytes: &[u8]) -> Result<narrata_history::ObjectId, HistoryError> {
    decode_single_id("Program index value", bytes).map(narrata_history::ObjectId::from_bytes)
}

// commits

pub(crate) fn commit_key(execution: ExecutionId, turn: u64, commit: CommitId) -> Vec<u8> {
    concat(&[execution.as_bytes(), &turn.to_be_bytes(), commit.as_bytes()])
}

pub(crate) fn decode_commit_key(
    bytes: &[u8],
) -> Result<(ExecutionId, u64, CommitId), HistoryError> {
    let (execution, rest) = split("commit key", bytes, 16)?;
    let (turn, commit) = split("commit key", rest, 8)?;
    Ok((
        ExecutionId::from_bytes(fixed("commit key", execution)?),
        u64::from_be_bytes(fixed("commit key", turn)?),
        CommitId::from_bytes(fixed("commit key", commit)?),
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rejects(result: Result<impl std::fmt::Debug, HistoryError>) {
        assert!(
            matches!(result, Err(HistoryError::CorruptStore(_))),
            "{result:?}"
        );
    }

    #[test]
    fn values_round_trip_and_reject_noncanonical_or_trailing_bytes() {
        let manifest = TimelineArchiveManifestId::from_bytes([7; 32]);
        let encoded = encode_archive(manifest);
        assert_eq!(decode_archive(&encoded).ok(), Some(manifest));
        let mut trailing = encoded.clone();
        trailing.push(0);
        rejects(decode_archive(&trailing));
        // Key 0 written as a one-byte-argument integer is valid CBOR but not canonical.
        let mut noncanonical = vec![0xa1, 0x18, 0x00];
        noncanonical.extend_from_slice(&encoded[2..]);
        rejects(decode_archive(&noncanonical));
        rejects(decode_fence(&encoded));
        assert_eq!(
            decode_fence(&encode_fence(LedgerFence::from_u64(42))).ok(),
            Some(LedgerFence::from_u64(42))
        );
    }

    #[test]
    fn coverage_and_ledger_values_round_trip() {
        let event = TimelineCatalogEventId::from_bytes([1; 32]);
        for coverage in [
            TimelineCoverage::FromBaseline {
                baseline: CommitId::from_bytes([2; 32]),
            },
            TimelineCoverage::Imported {
                baseline: CommitId::from_bytes([2; 32]),
                source: TimelineArchiveManifestId::from_bytes([3; 32]),
            },
        ] {
            let bytes = encode_catalog_head(event, coverage);
            assert_eq!(decode_catalog_head(&bytes).ok(), Some((event, coverage)));
        }
        let execution = ExecutionId::from_u128(9);
        let effect = EffectId::from_bytes([4; 32]);
        let capability = CapabilityId::new("host.query").unwrap();
        let statuses = [
            LedgerStatus::Claimed {
                lease: LeaseId::from_u128(5),
                expires_at: 30,
            },
            LedgerStatus::Completed {
                response: ObjectId::from_bytes([6; 32]),
                fence: LedgerFence::from_u64(1),
            },
            LedgerStatus::Rejected {
                response: ObjectId::from_bytes([6; 32]),
                fence: LedgerFence::from_u64(2),
            },
            LedgerStatus::RetryableFailure {
                diagnostic: DiagnosticId::from_bytes([8; 32]),
            },
            LedgerStatus::UnknownOutcome {
                diagnostic: DiagnosticId::from_bytes([8; 32]),
                fence: LedgerFence::from_u64(3),
            },
            LedgerStatus::Compensated {
                response: ObjectId::from_bytes([6; 32]),
                original_fence: LedgerFence::from_u64(1),
                by_effect: EffectId::from_bytes([9; 32]),
                compensation_fence: LedgerFence::from_u64(4),
            },
        ];
        let rewinds = [
            RewindPolicy::Reapply,
            RewindPolicy::ReuseRecordedResponse,
            RewindPolicy::Barrier,
            RewindPolicy::Compensatable {
                capability: capability.clone(),
            },
        ];
        for (status, rewind) in statuses.into_iter().zip(rewinds.into_iter().cycle()) {
            let entry = EffectLedgerEntry {
                execution,
                effect,
                request_digest: EffectRequestDigest::from_bytes([10; 32]),
                capability: capability.clone(),
                capability_version: CapabilityVersion::new(1).unwrap(),
                origin_commit: CommitId::from_bytes([11; 32]),
                delivery: DeliveryPolicy::RecordedQuery,
                rewind,
                status,
            };
            let key = effect_key(execution, effect);
            let value = encode_effect(&entry);
            assert_eq!(decode_effect(&key, &value).ok(), Some(entry));
        }
        let zero_fence = encode_effect(&EffectLedgerEntry {
            execution,
            effect,
            request_digest: EffectRequestDigest::from_bytes([10; 32]),
            capability: capability.clone(),
            capability_version: CapabilityVersion::new(1).unwrap(),
            origin_commit: CommitId::from_bytes([11; 32]),
            delivery: DeliveryPolicy::Reconcile,
            rewind: RewindPolicy::Reapply,
            status: LedgerStatus::Completed {
                response: ObjectId::from_bytes([6; 32]),
                fence: LedgerFence::zero(),
            },
        });
        rejects(decode_effect(&effect_key(execution, effect), &zero_fence));
    }

    #[test]
    fn keys_round_trip_and_keep_tuple_order() {
        let name = |value: &str| RefName::new(value).unwrap();
        let archive = TimelineArchiveRefKey::new(ExecutionId::from_u128(1), name("x"));
        assert_eq!(
            decode_archive_key(&archive_key(&archive)).ok(),
            Some(archive)
        );
        let compound = CompoundSaveRefKey::new(name("o"), name("s"));
        assert_eq!(
            decode_compound_save_key(&compound_save_key(&compound)).ok(),
            Some(compound)
        );
        let commit = CommitId::from_bytes([5; 32]);
        assert_eq!(
            decode_commit_key(&commit_key(ExecutionId::from_u128(2), 7, commit)).ok(),
            Some((ExecutionId::from_u128(2), 7, commit))
        );
        assert!(
            commit_key(ExecutionId::from_u128(2), 255, commit)
                < commit_key(ExecutionId::from_u128(2), 256, commit)
        );
        rejects(decode_commit_key(&[0; 55]));
    }
}
