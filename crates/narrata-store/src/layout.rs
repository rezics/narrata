//! Key-space layout v1 of the save engine (ADR 0014).
//!
//! Keys are raw fixed-width identities and `RefName` bytes; values are canonical CBOR maps.
//! Every decoder here is strict: it rejects trailing bytes, unknown shapes and any encoding that
//! does not re-encode to the same bytes, because the backend is outside the trust boundary.

use narrata_core::{
    CapabilityId, CapabilityVersion, CommitId, CompoundSaveManifestId, DeliveryPolicy,
    DiagnosticId, EffectId, EffectRequestDigest, ExecutionId, InputId, InputPayloadDigest,
    ObjectId, ProgramArtifactId, RewindPolicy, TimelineArchiveManifestId, TimelineCatalogEventId,
};
use narrata_storage::KeySpace;

use crate::{
    CatalogRefKey, CompoundSaveRefKey, EffectLedgerEntry, LeaseId, LedgerFence, LedgerStatus, Pin,
    RefKey, RefName, RefNamespace, StoreError, TimelineArchiveRefKey, TimelineCoverage,
    TimelineOperationId, WireError,
    codec::{Reader, Writer, expect_map, key},
};

/// Version recorded under `meta/layout`.
pub const LAYOUT_VERSION: u64 = 1;

pub const META: KeySpace = KeySpace::new(0);
pub const TOUCH: KeySpace = KeySpace::new(1);
pub const REFS: KeySpace = KeySpace::new(2);
pub const CATALOG_HEADS: KeySpace = KeySpace::new(3);
pub const ARCHIVES: KeySpace = KeySpace::new(4);
pub const COMPOUND_SAVES: KeySpace = KeySpace::new(5);
pub const INPUTS: KeySpace = KeySpace::new(6);
pub const PINS: KeySpace = KeySpace::new(7);
pub const EFFECTS: KeySpace = KeySpace::new(8);
pub const LEDGER_FENCES: KeySpace = KeySpace::new(9);
pub const CATALOG_OPERATIONS: KeySpace = KeySpace::new(10);
pub const PROGRAMS: KeySpace = KeySpace::new(11);
pub const CHILDREN: KeySpace = KeySpace::new(12);
pub const COMMITS: KeySpace = KeySpace::new(13);

/// Every space of the layout with its name, in space order.
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

pub(crate) const LAYOUT_KEY: &[u8] = b"layout";
/// Bumped by every batch that adds objects or roots; GC deletion batches check it.
pub(crate) const GRAPH_KEY: &[u8] = b"graph";
/// Bumped by every GC deletion batch; writers check it.
pub(crate) const SWEEP_KEY: &[u8] = b"sweep";

const SEPARATOR: u8 = 0;

fn corrupt(what: &str, error: impl std::fmt::Display) -> StoreError {
    StoreError::CorruptStore(format!("{what}: {error}"))
}

fn decode<T>(
    what: &'static str,
    bytes: &[u8],
    read: impl FnOnce(&mut Reader<'_>) -> Result<T, WireError>,
    encode: impl FnOnce(&T) -> Vec<u8>,
) -> Result<T, StoreError> {
    let mut reader = Reader::new(bytes);
    let value = read(&mut reader)
        .and_then(|value| reader.finish().map(|()| value))
        .map_err(|error| corrupt(what, error))?;
    if encode(&value) == bytes {
        Ok(value)
    } else {
        Err(corrupt(what, WireError::NonCanonical))
    }
}

fn fixed<const N: usize>(what: &'static str, bytes: &[u8]) -> Result<[u8; N], StoreError> {
    bytes
        .try_into()
        .map_err(|_| corrupt(what, "wrong key length"))
}

fn split<'a>(
    what: &'static str,
    bytes: &'a [u8],
    at: usize,
) -> Result<(&'a [u8], &'a [u8]), StoreError> {
    if bytes.len() < at {
        return Err(corrupt(what, "key is truncated"));
    }
    Ok(bytes.split_at(at))
}

fn name(what: &'static str, bytes: &[u8]) -> Result<RefName, StoreError> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|value| RefName::new(value).ok())
        .ok_or_else(|| corrupt(what, "invalid name"))
}

fn names(what: &'static str, bytes: &[u8]) -> Result<(RefName, RefName), StoreError> {
    let position = bytes
        .iter()
        .position(|byte| *byte == SEPARATOR)
        .ok_or_else(|| corrupt(what, "missing separator"))?;
    Ok((
        name(what, &bytes[..position])?,
        name(what, &bytes[position + 1..])?,
    ))
}

fn named(first: &[u8], second: &[u8]) -> Vec<u8> {
    let mut key = Vec::with_capacity(first.len() + second.len() + 1);
    key.extend_from_slice(first);
    key.push(SEPARATOR);
    key.extend_from_slice(second);
    key
}

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn empty_map() -> Vec<u8> {
    let mut writer = Writer::new();
    writer.map(0);
    writer.into_bytes()
}

fn single_bytes(value: &[u8]) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.map(1);
    writer.unsigned(0);
    writer.bytes(value);
    writer.into_bytes()
}

fn decode_single_id(what: &'static str, bytes: &[u8]) -> Result<[u8; 32], StoreError> {
    decode(
        what,
        bytes,
        |reader| {
            expect_map(reader, 1)?;
            key(reader, 0)?;
            reader.bytes_exact::<32>()
        },
        |value| single_bytes(value),
    )
}

fn single_unsigned(value: u64) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.map(1);
    writer.unsigned(0);
    writer.unsigned(value);
    writer.into_bytes()
}

fn decode_single_unsigned(what: &'static str, bytes: &[u8]) -> Result<u64, StoreError> {
    decode(
        what,
        bytes,
        |reader| {
            expect_map(reader, 1)?;
            key(reader, 0)?;
            reader.unsigned()
        },
        |value| single_unsigned(*value),
    )
}

// meta

pub(crate) fn encode_layout() -> Vec<u8> {
    single_unsigned(LAYOUT_VERSION)
}

pub(crate) fn decode_layout(bytes: &[u8]) -> Result<u64, StoreError> {
    decode_single_unsigned("layout version", bytes)
}

pub(crate) fn encode_marker() -> Vec<u8> {
    empty_map()
}

pub(crate) fn decode_marker(what: &'static str, bytes: &[u8]) -> Result<(), StoreError> {
    decode(
        what,
        bytes,
        |reader| expect_map(reader, 0),
        |()| empty_map(),
    )
}

// touch

pub(crate) fn touch_key(object: ObjectId) -> Vec<u8> {
    object.as_bytes().to_vec()
}

pub(crate) fn decode_touch_key(bytes: &[u8]) -> Result<ObjectId, StoreError> {
    fixed::<32>("touch key", bytes).map(ObjectId::from_bytes)
}

pub(crate) fn encode_touch(observed_at: u64) -> Vec<u8> {
    single_unsigned(observed_at)
}

pub(crate) fn decode_touch(bytes: &[u8]) -> Result<u64, StoreError> {
    decode_single_unsigned("touch value", bytes)
}

// refs

pub(crate) fn ref_key(key: &RefKey) -> Vec<u8> {
    let mut bytes = vec![key.namespace() as u8];
    bytes.extend(named(
        key.owner().as_str().as_bytes(),
        key.name().as_str().as_bytes(),
    ));
    bytes
}

pub(crate) fn ref_prefix(namespace: Option<RefNamespace>, owner: Option<&RefName>) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let Some(namespace) = namespace {
        bytes.push(namespace as u8);
        if let Some(owner) = owner {
            bytes.extend_from_slice(owner.as_str().as_bytes());
            bytes.push(SEPARATOR);
        }
    }
    bytes
}

pub(crate) fn decode_ref_key(bytes: &[u8]) -> Result<RefKey, StoreError> {
    let (namespace, rest) = split("Ref key", bytes, 1)?;
    let namespace =
        RefNamespace::from_code(namespace[0]).ok_or_else(|| corrupt("Ref key", "namespace"))?;
    let (owner, name) = names("Ref key", rest)?;
    Ok(RefKey::new(namespace, owner, name))
}

pub(crate) fn encode_commit_target(commit: CommitId) -> Vec<u8> {
    single_bytes(commit.as_bytes())
}

pub(crate) fn decode_commit_target(bytes: &[u8]) -> Result<CommitId, StoreError> {
    decode_single_id("Ref value", bytes).map(CommitId::from_bytes)
}

// catalog heads

pub(crate) fn catalog_key(key: &CatalogRefKey) -> Vec<u8> {
    key.timeline().as_bytes().to_vec()
}

pub(crate) fn decode_catalog_key(bytes: &[u8]) -> Result<CatalogRefKey, StoreError> {
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
) -> Result<(TimelineCatalogEventId, TimelineCoverage), StoreError> {
    decode(
        "Catalog Head value",
        bytes,
        |reader| {
            expect_map(reader, 2)?;
            key(reader, 0)?;
            let event = TimelineCatalogEventId::from_bytes(reader.bytes_exact::<32>()?);
            key(reader, 1)?;
            let length = reader.array()?;
            let coverage = match (length, reader.unsigned()?) {
                (2, 0) => TimelineCoverage::FromBaseline {
                    baseline: CommitId::from_bytes(reader.bytes_exact::<32>()?),
                },
                (3, 1) => TimelineCoverage::Imported {
                    baseline: CommitId::from_bytes(reader.bytes_exact::<32>()?),
                    source: TimelineArchiveManifestId::from_bytes(reader.bytes_exact::<32>()?),
                },
                _ => return Err(WireError::Schema("Catalog coverage")),
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

pub(crate) fn decode_archive_key(bytes: &[u8]) -> Result<TimelineArchiveRefKey, StoreError> {
    let (timeline, rest) = split("Archive key", bytes, 16)?;
    Ok(TimelineArchiveRefKey::new(
        ExecutionId::from_bytes(fixed("Archive key", timeline)?),
        name("Archive key", rest)?,
    ))
}

pub(crate) fn encode_archive(manifest: TimelineArchiveManifestId) -> Vec<u8> {
    single_bytes(manifest.as_bytes())
}

pub(crate) fn decode_archive(bytes: &[u8]) -> Result<TimelineArchiveManifestId, StoreError> {
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

pub(crate) fn decode_compound_save_key(bytes: &[u8]) -> Result<CompoundSaveRefKey, StoreError> {
    let (owner, slot) = names("Compound Save key", bytes)?;
    Ok(CompoundSaveRefKey::new(owner, slot))
}

pub(crate) fn encode_compound_save(manifest: CompoundSaveManifestId) -> Vec<u8> {
    single_bytes(manifest.as_bytes())
}

pub(crate) fn decode_compound_save(bytes: &[u8]) -> Result<CompoundSaveManifestId, StoreError> {
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
) -> Result<(CommitId, InputPayloadDigest, CommitId), StoreError> {
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

// pins

/// Pin owners are free text, so they may not contain the key separator.
pub(crate) fn pin_key(owner: &str, object: ObjectId) -> Result<Vec<u8>, StoreError> {
    if owner.as_bytes().contains(&SEPARATOR) {
        return Err(StoreError::InvalidGraph("Pin owner contains NUL"));
    }
    Ok(named(owner.as_bytes(), object.as_bytes()))
}

pub(crate) fn decode_pin_key(bytes: &[u8]) -> Result<(String, ObjectId), StoreError> {
    let owner_length = bytes
        .len()
        .checked_sub(33)
        .ok_or_else(|| corrupt("Pin key", "key is truncated"))?;
    let (owner, rest) = bytes.split_at(owner_length);
    let (separator, object) = rest.split_at(1);
    if separator != [SEPARATOR] || owner.contains(&SEPARATOR) {
        return Err(corrupt("Pin key", "misplaced separator"));
    }
    let owner = std::str::from_utf8(owner).map_err(|error| corrupt("Pin key", error))?;
    Ok((
        owner.to_owned(),
        ObjectId::from_bytes(fixed("Pin key", object)?),
    ))
}

pub(crate) fn encode_pin(expires_at: Option<u64>) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.map(1);
    writer.unsigned(0);
    match expires_at {
        Some(value) => writer.unsigned(value),
        None => writer.null(),
    }
    writer.into_bytes()
}

pub(crate) fn decode_pin(key_bytes: &[u8], bytes: &[u8]) -> Result<Pin, StoreError> {
    let (owner, object) = decode_pin_key(key_bytes)?;
    let expires_at = decode(
        "Pin value",
        bytes,
        |reader| {
            expect_map(reader, 1)?;
            key(reader, 0)?;
            reader.optional(Reader::unsigned)
        },
        |value| encode_pin(*value),
    )?;
    Ok(Pin {
        owner,
        object,
        expires_at,
    })
}

// effects

pub(crate) fn effect_key(execution: ExecutionId, effect: EffectId) -> Vec<u8> {
    concat(&[execution.as_bytes(), effect.as_bytes()])
}

pub(crate) fn decode_effect_key(bytes: &[u8]) -> Result<(ExecutionId, EffectId), StoreError> {
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

fn capability(reader: &mut Reader<'_>) -> Result<CapabilityId, WireError> {
    CapabilityId::new(reader.text(64)?).map_err(|_| WireError::Schema("Effect capability"))
}

fn terminal_fence(reader: &mut Reader<'_>) -> Result<LedgerFence, WireError> {
    match reader.unsigned()? {
        0 => Err(WireError::Schema("terminal ledger fence is zero")),
        value => Ok(LedgerFence::from_u64(value)),
    }
}

pub(crate) fn decode_effect(
    key_bytes: &[u8],
    bytes: &[u8],
) -> Result<EffectLedgerEntry, StoreError> {
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
                u16::try_from(reader.unsigned()?).map_err(|_| WireError::IntegerOverflow)?;
            let capability_version = CapabilityVersion::new(version)
                .ok_or(WireError::Schema("Effect capability version"))?;
            key(reader, 3)?;
            let origin_commit = CommitId::from_bytes(reader.bytes_exact::<32>()?);
            key(reader, 4)?;
            let delivery = delivery_from_code(reader.unsigned()?)
                .ok_or(WireError::Schema("Effect delivery policy"))?;
            key(reader, 5)?;
            let length = reader.array()?;
            let rewind = match (length, reader.unsigned()?) {
                (1, 0) => RewindPolicy::Reapply,
                (1, 1) => RewindPolicy::ReuseRecordedResponse,
                (1, 2) => RewindPolicy::Barrier,
                (2, 3) => RewindPolicy::Compensatable {
                    capability: capability(reader)?,
                },
                _ => return Err(WireError::Schema("Effect rewind policy")),
            };
            key(reader, 6)?;
            let length = reader.array()?;
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
                _ => return Err(WireError::Schema("Effect status")),
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

pub(crate) fn decode_fence(bytes: &[u8]) -> Result<LedgerFence, StoreError> {
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
) -> Result<(Option<TimelineCatalogEventId>, TimelineCatalogEventId), StoreError> {
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

pub(crate) fn encode_program(object: ObjectId) -> Vec<u8> {
    single_bytes(object.as_bytes())
}

pub(crate) fn decode_program(bytes: &[u8]) -> Result<ObjectId, StoreError> {
    decode_single_id("Program index value", bytes).map(ObjectId::from_bytes)
}

// children

pub(crate) fn child_key(parent: CommitId, child: CommitId) -> Vec<u8> {
    concat(&[parent.as_bytes(), child.as_bytes()])
}

pub(crate) fn decode_child_key(bytes: &[u8]) -> Result<(CommitId, CommitId), StoreError> {
    let (parent, child) = split("child key", bytes, 32)?;
    Ok((
        CommitId::from_bytes(fixed("child key", parent)?),
        CommitId::from_bytes(fixed("child key", child)?),
    ))
}

// commits

pub(crate) fn commit_key(execution: ExecutionId, turn: u64, commit: CommitId) -> Vec<u8> {
    concat(&[execution.as_bytes(), &turn.to_be_bytes(), commit.as_bytes()])
}

pub(crate) fn decode_commit_key(bytes: &[u8]) -> Result<(ExecutionId, u64, CommitId), StoreError> {
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

    fn rejects(result: Result<impl std::fmt::Debug, StoreError>) {
        assert!(
            matches!(result, Err(StoreError::CorruptStore(_))),
            "{result:?}"
        );
    }

    #[test]
    fn values_round_trip_and_reject_noncanonical_or_trailing_bytes() {
        let commit = CommitId::from_bytes([7; 32]);
        let encoded = encode_commit_target(commit);
        assert_eq!(decode_commit_target(&encoded).ok(), Some(commit));
        let mut trailing = encoded.clone();
        trailing.push(0);
        rejects(decode_commit_target(&trailing));
        // Key 0 written as a one-byte-argument integer is valid CBOR but not canonical.
        let mut noncanonical = vec![0xa1, 0x18, 0x00];
        noncanonical.extend_from_slice(&encoded[2..]);
        rejects(decode_commit_target(&noncanonical));
        rejects(decode_touch(&encode_commit_target(commit)));
        assert_eq!(decode_touch(&encode_touch(42)).ok(), Some(42));
        assert_eq!(decode_layout(&encode_layout()).ok(), Some(LAYOUT_VERSION));
        assert!(decode_marker("graph", &encode_marker()).is_ok());
        rejects(decode_marker("graph", &encode_touch(0)));
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
        let short = RefKey::save(name("a"), name("z"));
        let long = RefKey::save(name("ab"), name("a"));
        assert!(ref_key(&short) < ref_key(&long));
        assert_eq!(decode_ref_key(&ref_key(&long)).ok(), Some(long));
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
        let object = ObjectId::from_bytes([3; 32]);
        let pin = pin_key("owner", object).unwrap();
        assert_eq!(
            decode_pin_key(&pin).ok(),
            Some(("owner".to_owned(), object))
        );
        assert!(pin_key("a\0b", object).is_err());
        assert!(pin_key("a", object).unwrap() < pin_key("ab", object).unwrap());
        let commit = CommitId::from_bytes([5; 32]);
        assert_eq!(
            decode_commit_key(&commit_key(ExecutionId::from_u128(2), 7, commit)).ok(),
            Some((ExecutionId::from_u128(2), 7, commit))
        );
        assert!(
            commit_key(ExecutionId::from_u128(2), 255, commit)
                < commit_key(ExecutionId::from_u128(2), 256, commit)
        );
        rejects(decode_ref_key(&[9, b'a', 0, b'b']));
        rejects(decode_ref_key(&[0, b'a', b'b']));
        rejects(decode_child_key(&[0; 63]));
    }
}
