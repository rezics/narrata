use crate::limits::DecodeLimits;

use super::DecodeError;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u16)]
pub enum ObjectKind {
    Program = 1,
    Snapshot = 2,
    Receipt = 3,
    Value = 4,
    Commit = 5,
    TimelineCatalogEvent = 6,
    CheckpointBundleManifest = 7,
    TimelineArchiveManifest = 8,
    EffectResponse = 9,
    CompoundSaveManifest = 10,
    HostTimelineManifest = 11,
}

impl ObjectKind {
    pub const fn from_code(value: u16) -> Option<Self> {
        match value {
            1 => Some(Self::Program),
            2 => Some(Self::Snapshot),
            3 => Some(Self::Receipt),
            4 => Some(Self::Value),
            5 => Some(Self::Commit),
            6 => Some(Self::TimelineCatalogEvent),
            7 => Some(Self::CheckpointBundleManifest),
            8 => Some(Self::TimelineArchiveManifest),
            9 => Some(Self::EffectResponse),
            10 => Some(Self::CompoundSaveManifest),
            11 => Some(Self::HostTimelineManifest),
            _ => None,
        }
    }

    pub const fn code(self) -> u16 {
        self as u16
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Envelope<'a> {
    pub kind: ObjectKind,
    pub schema_version: u16,
    pub payload: &'a [u8],
}

pub fn encode_envelope(kind: ObjectKind, schema_version: u16, payload: &[u8]) -> Vec<u8> {
    narrata_kernel::codec::encode_envelope(kind.code(), schema_version, payload)
}

pub fn decode_envelope<'a>(
    bytes: &'a [u8],
    expected_kind: ObjectKind,
    expected_schema: u16,
    limits: &DecodeLimits,
) -> Result<Envelope<'a>, DecodeError> {
    decode_envelope_versions(bytes, expected_kind, &[expected_schema], limits)
}

pub fn decode_envelope_versions<'a>(
    bytes: &'a [u8],
    expected_kind: ObjectKind,
    supported_schemas: &[u16],
    limits: &DecodeLimits,
) -> Result<Envelope<'a>, DecodeError> {
    let envelope = inspect_envelope(bytes, limits)?;
    if envelope.kind != expected_kind {
        return Err(DecodeError::Envelope("wrong object kind"));
    }
    if !supported_schemas.contains(&envelope.schema_version) {
        return Err(DecodeError::UnsupportedVersion {
            axis: "object schema",
            version: envelope.schema_version,
        });
    }
    Ok(envelope)
}

pub fn inspect_envelope<'a>(
    bytes: &'a [u8],
    limits: &DecodeLimits,
) -> Result<Envelope<'a>, DecodeError> {
    let envelope = narrata_kernel::codec::inspect_envelope_with_kind(
        bytes,
        &narrata_kernel::codec::EnvelopeLimits {
            max_envelope_bytes: limits.max_envelope_bytes,
            max_payload_bytes: limits.max_payload_bytes,
        },
        |code| {
            ObjectKind::from_code(code)
                .map(|_| ())
                .ok_or(DecodeError::Envelope("unknown object kind"))
        },
    )?;
    Ok(Envelope {
        kind: ObjectKind::from_code(envelope.kind)
            .ok_or(DecodeError::Envelope("unknown object kind"))?,
        schema_version: envelope.schema_version,
        payload: envelope.payload,
    })
}
