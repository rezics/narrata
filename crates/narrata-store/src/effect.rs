use narrata_core::{
    CapabilityId, CapabilityVersion, CommitId, DiagnosticId, EffectId, EffectRequestDigest,
    ExecutionId, ObjectId, RewindPolicy, Value,
    codec::{ObjectKind, decode_canonical_value, encode_canonical_value},
    limits::DecodeLimits,
};

use crate::{
    CheckedObject, LeaseId, WireError,
    codec::{Reader, Writer, expect_map, key},
};

pub const EFFECT_RESPONSE_SCHEMA_V1: u16 = 1;

pub use narrata_history::effect::{EffectStoreError, LedgerFence};

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
    pub capability: CapabilityId,
    pub capability_version: CapabilityVersion,
    pub origin_commit: CommitId,
    pub delivery: narrata_core::DeliveryPolicy,
    pub rewind: RewindPolicy,
    pub status: LedgerStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectClaim {
    pub execution: ExecutionId,
    pub effect: EffectId,
    pub request_digest: EffectRequestDigest,
    pub capability: CapabilityId,
    pub capability_version: CapabilityVersion,
    pub origin_commit: CommitId,
    pub delivery: narrata_core::DeliveryPolicy,
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
pub enum EffectOutcome {
    Completed(RecordedEffectResponseV1),
    Rejected(RecordedEffectResponseV1),
    RetryableFailure(DiagnosticId),
    UnknownOutcome(DiagnosticId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectOutcomeRecord {
    pub execution: ExecutionId,
    pub effect: EffectId,
    pub request_digest: EffectRequestDigest,
    pub lease: LeaseId,
    pub outcome: EffectOutcome,
    pub observed_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedEffectResponseV1 {
    pub effect: EffectId,
    pub request_digest: EffectRequestDigest,
    pub capability: CapabilityId,
    pub capability_version: CapabilityVersion,
    pub payload: Value,
}

impl RecordedEffectResponseV1 {
    pub fn to_object(&self) -> CheckedObject {
        CheckedObject::new(
            ObjectKind::EffectResponse,
            EFFECT_RESPONSE_SCHEMA_V1,
            &self.encode(),
        )
    }

    pub fn encode(&self) -> Vec<u8> {
        let payload = encode_canonical_value(&self.payload);
        let mut writer = Writer::new();
        writer.map(6);
        writer.unsigned(0);
        writer.unsigned(u64::from(EFFECT_RESPONSE_SCHEMA_V1));
        writer.unsigned(1);
        writer.bytes(self.effect.as_bytes());
        writer.unsigned(2);
        writer.bytes(self.request_digest.as_bytes());
        writer.unsigned(3);
        writer.text(self.capability.as_str());
        writer.unsigned(4);
        writer.unsigned(u64::from(self.capability_version.get()));
        writer.unsigned(5);
        writer.bytes(&payload);
        writer.into_bytes()
    }

    pub fn decode(encoded: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(encoded);
        expect_map(&mut reader, 6)?;
        key(&mut reader, 0)?;
        if reader.unsigned()? != u64::from(EFFECT_RESPONSE_SCHEMA_V1) {
            return Err(WireError::Schema("Effect response schema"));
        }
        key(&mut reader, 1)?;
        let effect = EffectId::from_bytes(reader.bytes_exact::<32>()?);
        key(&mut reader, 2)?;
        let request_digest = EffectRequestDigest::from_bytes(reader.bytes_exact::<32>()?);
        key(&mut reader, 3)?;
        let capability = CapabilityId::new(reader.text(64)?)
            .map_err(|_| WireError::Schema("Effect response capability"))?;
        key(&mut reader, 4)?;
        let version = u16::try_from(reader.unsigned()?).map_err(|_| WireError::IntegerOverflow)?;
        let capability_version = CapabilityVersion::new(version)
            .ok_or(WireError::Schema("Effect response capability version"))?;
        key(&mut reader, 5)?;
        let value_bytes = reader.bytes(16 * 1024 * 1024)?;
        let payload = decode_canonical_value(value_bytes, &DecodeLimits::default())
            .map_err(|_| WireError::Schema("Effect response payload"))?;
        reader.finish()?;
        let value = Self {
            effect,
            request_digest,
            capability,
            capability_version,
            payload,
        };
        if value.encode() != encoded {
            return Err(WireError::NonCanonical);
        }
        Ok(value)
    }
}
