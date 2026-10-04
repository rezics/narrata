use super::{DecodeError, sha256};

const MAGIC: &[u8; 8] = b"NARRATA\0";
const HEADER_LENGTH: usize = 56;
const ENVELOPE_V0: u16 = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnvelopeLimits {
    pub max_envelope_bytes: u64,
    pub max_payload_bytes: u64,
}

impl Default for EnvelopeLimits {
    fn default() -> Self {
        Self {
            max_envelope_bytes: 16 * 1024 * 1024,
            max_payload_bytes: 16 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Envelope<'a> {
    pub kind: u16,
    pub schema_version: u16,
    pub payload: &'a [u8],
}

pub fn encode_envelope(kind: u16, schema_version: u16, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER_LENGTH.saturating_add(payload.len()));
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&ENVELOPE_V0.to_be_bytes());
    bytes.extend_from_slice(&kind.to_be_bytes());
    bytes.extend_from_slice(&schema_version.to_be_bytes());
    bytes.extend_from_slice(&0_u16.to_be_bytes());
    bytes.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    bytes.extend_from_slice(&sha256(payload));
    bytes.extend_from_slice(payload);
    bytes
}

pub fn decode_envelope<'a>(
    bytes: &'a [u8],
    expected_kind: u16,
    expected_schema: u16,
    limits: &EnvelopeLimits,
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
    expected_kind: u16,
    supported_schemas: &[u16],
    limits: &EnvelopeLimits,
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
    limits: &EnvelopeLimits,
) -> Result<Envelope<'a>, DecodeError> {
    inspect_envelope_with_kind(bytes, limits, |_| Ok(()))
}

/// Allows a domain to reject kind codes before validating the payload.
pub fn inspect_envelope_with_kind<'a>(
    bytes: &'a [u8],
    limits: &EnvelopeLimits,
    validate_kind: impl FnOnce(u16) -> Result<(), DecodeError>,
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
    if envelope_version != ENVELOPE_V0 {
        return Err(DecodeError::UnsupportedVersion {
            axis: "envelope",
            version: envelope_version,
        });
    }
    let kind = read_u16(header, 10)?;
    validate_kind(kind)?;
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
