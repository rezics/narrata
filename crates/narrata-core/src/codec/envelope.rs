use crate::{limits::DecodeLimits, version::ENVELOPE_V0};

use super::{DecodeError, sha256};

const MAGIC: &[u8; 8] = b"NARRATA\0";
const HEADER_LENGTH: usize = 56;

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
    let mut bytes = Vec::with_capacity(HEADER_LENGTH.saturating_add(payload.len()));
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&ENVELOPE_V0.get().to_be_bytes());
    bytes.extend_from_slice(&(kind as u16).to_be_bytes());
    bytes.extend_from_slice(&schema_version.to_be_bytes());
    bytes.extend_from_slice(&0_u16.to_be_bytes());
    bytes.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    bytes.extend_from_slice(&sha256(payload));
    bytes.extend_from_slice(payload);
    bytes
}

pub fn decode_envelope<'a>(
    bytes: &'a [u8],
    expected_kind: ObjectKind,
    expected_schema: u16,
    limits: &DecodeLimits,
) -> Result<Envelope<'a>, DecodeError> {
    let envelope = inspect_envelope(bytes, limits)?;
    if envelope.kind != expected_kind {
        return Err(DecodeError::Envelope("wrong object kind"));
    }
    if envelope.schema_version != expected_schema {
        return Err(DecodeError::UnsupportedVersion {
            axis: "object schema",
            version: envelope.schema_version,
        });
    }
    Ok(envelope)
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
    if bytes.len() as u64 > limits.max_envelope_bytes {
        return Err(DecodeError::Limit("envelope bytes"));
    }
    let header = bytes
        .get(..HEADER_LENGTH)
        .ok_or(DecodeError::Envelope("truncated header"))?;
    if header.get(..8) != Some(MAGIC) {
        return Err(DecodeError::Envelope("wrong magic"));
    }
    let envelope_version = read_u16(header, 8)?;
    if envelope_version != ENVELOPE_V0.get() {
        return Err(DecodeError::UnsupportedVersion {
            axis: "envelope",
            version: envelope_version,
        });
    }
    let kind = ObjectKind::from_code(read_u16(header, 10)?)
        .ok_or(DecodeError::Envelope("unknown object kind"))?;
    let schema_version = read_u16(header, 12)?;
    if read_u16(header, 14)? != 0 {
        return Err(DecodeError::Envelope("unsupported flags"));
    }
    let payload_length = read_u64(header, 16)?;
    if payload_length > limits.max_payload_bytes {
        return Err(DecodeError::Limit("payload bytes"));
    }
    let payload_length =
        usize::try_from(payload_length).map_err(|_| DecodeError::LengthOverflow)?;
    let expected_total = HEADER_LENGTH
        .checked_add(payload_length)
        .ok_or(DecodeError::LengthOverflow)?;
    if bytes.len() != expected_total {
        return Err(DecodeError::Envelope(
            "payload length mismatch or trailing bytes",
        ));
    }
    let payload = bytes
        .get(HEADER_LENGTH..)
        .ok_or(DecodeError::Envelope("missing payload"))?;
    let expected_digest = header
        .get(24..56)
        .ok_or(DecodeError::Envelope("missing payload digest"))?;
    if sha256(payload).as_slice() != expected_digest {
        return Err(DecodeError::Envelope("payload digest mismatch"));
    }
    Ok(Envelope {
        kind,
        schema_version,
        payload,
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, DecodeError> {
    let value = bytes
        .get(offset..offset.saturating_add(2))
        .ok_or(DecodeError::Envelope("truncated u16"))?;
    Ok(u16::from_be_bytes([value[0], value[1]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, DecodeError> {
    let value = bytes
        .get(offset..offset.saturating_add(8))
        .ok_or(DecodeError::Envelope("truncated u64"))?;
    Ok(u64::from_be_bytes([
        value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7],
    ]))
}
