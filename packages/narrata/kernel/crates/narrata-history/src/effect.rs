//! Monotonic effect facts, independent of a domain's rollback state (ADR 0020).
use crate::{Domain, HistoryError, Object, ObjectId, Reader, Root, layout};
use narrata_kernel::codec::{CborReader, CborWriter, DecodeError};
use narrata_storage::{KeySpace, StorageBackend};
use thiserror::Error;

narrata_kernel::authored_id!(ExecutionId, "execution:");
narrata_kernel::derived_id!(EffectId, "effect:");
narrata_kernel::derived_id!(EffectRequestDigest, "effect-request:");
narrata_kernel::derived_id!(DiagnosticId, "diagnostic:");
narrata_kernel::authored_id!(LeaseId, "lease:");

pub const EFFECTS: KeySpace = KeySpace::new(16);
pub const LEDGER_FENCES: KeySpace = KeySpace::new(17);
pub const EFFECT_RESPONSE_KIND: u16 = 13;
/// Keeps pre-ledger history engines from collecting roots they cannot see.
pub const EFFECT_LEDGER_GUARD_KIND: u16 = 14;
pub fn ledger_guard() -> Object {
    Object::new(EFFECT_LEDGER_GUARD_KIND, 1, &layout::encode_marker())
}
pub(crate) fn validate_guard(object: &Object) -> Result<(), HistoryError> {
    if object.schema() != 1 || object.id() != ledger_guard().id() {
        return Err(HistoryError::ObjectKind(object.id()));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryPolicy {
    Reconcile,
    RecordedQuery,
    AtLeastOnceIdempotent,
    HostTransactional,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RewindPolicy {
    Reapply,
    ReuseRecordedResponse,
    Barrier,
    Compensatable { capability: Vec<u8> },
}
pub const EFFECT_RESPONSE_SCHEMA: u16 = 1;

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct LedgerFence(u64);

impl LedgerFence {
    pub const fn zero() -> Self {
        Self(0)
    }

    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn next(self) -> Result<Self, EffectStoreError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(EffectStoreError::FenceOverflow)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LedgerStatus {
    Claimed {
        lease: LeaseId,
        expires_at: u64,
    },
    Completed {
        response: ObjectId,
        fence: LedgerFence,
    },
    Rejected {
        response: ObjectId,
        fence: LedgerFence,
    },
    RetryableFailure {
        diagnostic: DiagnosticId,
    },
    UnknownOutcome {
        diagnostic: DiagnosticId,
        fence: LedgerFence,
    },
    Compensated {
        response: ObjectId,
        original_fence: LedgerFence,
        by_effect: EffectId,
        compensation_fence: LedgerFence,
    },
}

impl LedgerStatus {
    pub const fn fence(&self) -> Option<LedgerFence> {
        match self {
            Self::Completed { fence, .. }
            | Self::Rejected { fence, .. }
            | Self::UnknownOutcome { fence, .. } => Some(*fence),
            Self::Compensated {
                compensation_fence, ..
            } => Some(*compensation_fence),
            Self::Claimed { .. } | Self::RetryableFailure { .. } => None,
        }
    }

    pub const fn response(&self) -> Option<ObjectId> {
        match self {
            Self::Completed { response, .. }
            | Self::Rejected { response, .. }
            | Self::Compensated { response, .. } => Some(*response),
            Self::Claimed { .. } | Self::RetryableFailure { .. } | Self::UnknownOutcome { .. } => {
                None
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectLedgerEntry {
    pub execution: ExecutionId,
    pub effect: EffectId,
    pub request_digest: EffectRequestDigest,
    pub capability: Vec<u8>,
    pub capability_version: Vec<u8>,
    pub origin_commit: ObjectId,
    pub delivery: DeliveryPolicy,
    pub rewind: RewindPolicy,
    pub status: LedgerStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectClaim {
    pub execution: ExecutionId,
    pub effect: EffectId,
    pub request_digest: EffectRequestDigest,
    pub capability: Vec<u8>,
    pub capability_version: Vec<u8>,
    pub origin_commit: ObjectId,
    pub delivery: DeliveryPolicy,
    pub rewind: RewindPolicy,
    pub lease: LeaseId,
    pub now: u64,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EffectClaimResult {
    Claimed(EffectLedgerEntry),
    Recorded(EffectLedgerEntry),
    Leased { lease: LeaseId, expires_at: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EffectOutcome<R = RecordedEffectResponse> {
    Completed(R),
    Rejected(R),
    RetryableFailure(DiagnosticId),
    UnknownOutcome(DiagnosticId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectOutcomeRecord<R = RecordedEffectResponse> {
    pub execution: ExecutionId,
    pub effect: EffectId,
    pub request_digest: EffectRequestDigest,
    pub lease: LeaseId,
    pub outcome: EffectOutcome<R>,
    pub observed_at: u64,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum EffectStoreError {
    #[error("Effect ID is already bound to a different request digest or contract")]
    RequestConflict,
    #[error("Effect is not currently claimed by the supplied lease")]
    LeaseMismatch,
    #[error("lease expiration must be later than now")]
    InvalidLease,
    #[error("Effect outcome is terminal and cannot move backward")]
    Terminal,
    #[error("ledger fence overflow")]
    FenceOverflow,
    #[error("Effect ledger entry was not found")]
    Missing,
    #[error("compensation relation is invalid")]
    InvalidCompensation,
    #[error("recorded response object is invalid: {0}")]
    InvalidResponse(String),
}

/// The host's opaque response bytes; history never interprets the payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedEffectResponse {
    pub effect: EffectId,
    pub request_digest: EffectRequestDigest,
    pub capability: Vec<u8>,
    pub capability_version: Vec<u8>,
    pub payload: Vec<u8>,
}
impl RecordedEffectResponse {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = CborWriter::new();
        w.map(5);
        w.unsigned(0);
        w.bytes(self.effect.as_bytes());
        w.unsigned(1);
        w.bytes(self.request_digest.as_bytes());
        w.unsigned(2);
        w.bytes(&self.capability);
        w.unsigned(3);
        w.bytes(&self.capability_version);
        w.unsigned(4);
        w.bytes(&self.payload);
        w.into_bytes()
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, HistoryError> {
        layout::decode(
            "effect response",
            bytes,
            |r| {
                layout::expect_map(r, 5)?;
                layout::key(r, 0)?;
                let effect = EffectId::from_bytes(r.bytes_exact::<32>()?);
                layout::key(r, 1)?;
                let request_digest = EffectRequestDigest::from_bytes(r.bytes_exact::<32>()?);
                layout::key(r, 2)?;
                let capability = r.bytes(65536)?.to_vec();
                layout::key(r, 3)?;
                let capability_version = r.bytes(65536)?.to_vec();
                layout::key(r, 4)?;
                let payload = r.bytes(16 * 1024 * 1024)?.to_vec();
                Ok(Self {
                    effect,
                    request_digest,
                    capability,
                    capability_version,
                    payload,
                })
            },
            Self::encode,
        )
    }
    pub fn to_object(&self) -> Object {
        Object::new(EFFECT_RESPONSE_KIND, EFFECT_RESPONSE_SCHEMA, &self.encode())
    }
}

/// A registration preserves a domain's key/value and response formats while using one state
/// machine. The fixed-width execution/effect key and fence encoding are shared.
pub trait EffectRegistration {
    const EFFECTS: KeySpace;
    const FENCES: KeySpace;
    const COMMIT_KIND: u16;
    type Response;
    fn encode(entry: &EffectLedgerEntry) -> Result<Vec<u8>, HistoryError>;
    fn decode(key: &[u8], value: &[u8]) -> Result<EffectLedgerEntry, HistoryError>;
    fn response(
        entry: &EffectLedgerEntry,
        response: &Self::Response,
    ) -> Result<Object, EffectStoreError>;
}

/// The byte-contract ledger for a registered domain's generic commits.
pub struct DomainEffects<D>(std::marker::PhantomData<fn() -> D>);
impl<D: Domain> EffectRegistration for DomainEffects<D> {
    const EFFECTS: KeySpace = EFFECTS;
    const FENCES: KeySpace = LEDGER_FENCES;
    const COMMIT_KIND: u16 = D::COMMIT_KIND;
    type Response = RecordedEffectResponse;
    fn encode(entry: &EffectLedgerEntry) -> Result<Vec<u8>, HistoryError> {
        Ok(encode_effect(entry))
    }
    fn decode(key: &[u8], value: &[u8]) -> Result<EffectLedgerEntry, HistoryError> {
        decode_effect(key, value)
    }
    fn response(
        entry: &EffectLedgerEntry,
        response: &Self::Response,
    ) -> Result<Object, EffectStoreError> {
        if response.effect != entry.effect
            || response.request_digest != entry.request_digest
            || response.capability != entry.capability
            || response.capability_version != entry.capability_version
        {
            return Err(EffectStoreError::InvalidResponse(
                "contract mismatch".to_owned(),
            ));
        }
        Ok(response.to_object())
    }
}

pub fn effect_key(execution: ExecutionId, effect: EffectId) -> Vec<u8> {
    [execution.as_bytes().as_slice(), effect.as_bytes()].concat()
}
pub fn fence_key(execution: ExecutionId) -> Vec<u8> {
    execution.as_bytes().to_vec()
}
pub fn encode_fence(fence: LedgerFence) -> Vec<u8> {
    layout::single_unsigned(fence.get())
}
pub fn decode_fence(bytes: &[u8]) -> Result<LedgerFence, HistoryError> {
    layout::decode_single_unsigned("ledger fence value", bytes).map(LedgerFence::from_u64)
}
pub fn roots<B: StorageBackend, L: EffectRegistration>(
    reader: &Reader<'_, B>,
) -> Result<Vec<Root>, HistoryError> {
    let mut roots = Vec::new();
    for value in reader.scan_every(L::EFFECTS, &[])? {
        let entry = L::decode(&value.key, &value.value)?;
        roots.push((entry.origin_commit, Some(L::COMMIT_KIND)));
        roots.extend(entry.status.response().map(|id| (id, None)));
    }
    Ok(roots)
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

pub fn encode_effect(entry: &EffectLedgerEntry) -> Vec<u8> {
    let mut writer = CborWriter::new();
    writer.map(7);
    writer.unsigned(0);
    writer.bytes(entry.request_digest.as_bytes());
    writer.unsigned(1);
    writer.bytes(&entry.capability);
    writer.unsigned(2);
    writer.bytes(&entry.capability_version);
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
            writer.bytes(capability);
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

fn capability(reader: &mut CborReader<'_>) -> Result<Vec<u8>, DecodeError> {
    Ok(reader.bytes(65536)?.to_vec())
}
fn terminal_fence(reader: &mut CborReader<'_>) -> Result<LedgerFence, DecodeError> {
    match reader.unsigned()? {
        0 => Err(DecodeError::Schema("terminal ledger fence is zero")),
        value => Ok(LedgerFence::from_u64(value)),
    }
}

pub fn decode_effect(key_bytes: &[u8], bytes: &[u8]) -> Result<EffectLedgerEntry, HistoryError> {
    let bytes_key: [u8; 48] = layout::fixed("Effect key", key_bytes)?;
    let execution = ExecutionId::from_bytes(layout::fixed("Effect key", &bytes_key[..16])?);
    let effect = EffectId::from_bytes(layout::fixed("Effect key", &bytes_key[16..])?);
    layout::decode(
        "Effect value",
        bytes,
        |reader| {
            layout::expect_map(reader, 7)?;
            layout::key(reader, 0)?;
            let request_digest = EffectRequestDigest::from_bytes(reader.bytes_exact::<32>()?);
            layout::key(reader, 1)?;
            let capability_id = capability(reader)?;
            layout::key(reader, 2)?;
            let capability_version = reader.bytes(65536)?.to_vec();
            layout::key(reader, 3)?;
            let origin_commit = ObjectId::from_bytes(reader.bytes_exact::<32>()?);
            layout::key(reader, 4)?;
            let delivery = delivery_from_code(reader.unsigned()?)
                .ok_or(DecodeError::Schema("Effect delivery policy"))?;
            layout::key(reader, 5)?;
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
            layout::key(reader, 6)?;
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

pub(crate) fn generic_roots<B: StorageBackend>(
    reader: &Reader<'_, B>,
) -> Result<Vec<Root>, HistoryError> {
    let mut roots = Vec::new();
    for value in reader.scan_every(EFFECTS, &[])? {
        let entry = decode_effect(&value.key, &value.value)?;
        roots.push((entry.origin_commit, None));
        roots.extend(
            entry
                .status
                .response()
                .map(|id| (id, Some(EFFECT_RESPONSE_KIND))),
        );
    }
    if !roots.is_empty() {
        roots.push((ledger_guard().id(), Some(EFFECT_LEDGER_GUARD_KIND)));
    }
    Ok(roots)
}
pub(crate) fn validate_response_object(object: &Object) -> Result<(), HistoryError> {
    if object.schema() != EFFECT_RESPONSE_SCHEMA {
        return Err(HistoryError::ObjectKind(object.id()));
    }
    RecordedEffectResponse::decode(object.payload())
        .map(|_| ())
        .map_err(|e| HistoryError::Corrupt(object.id(), e.to_string()))
}
