use thiserror::Error;

use crate::{limits::DecodeLimits, value::Value};

use super::CborReader;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DecodeError {
    #[error("unexpected end of CBOR input")]
    UnexpectedEnd,
    #[error("trailing bytes after CBOR object")]
    TrailingBytes,
    #[error("CBOR length cannot be represented")]
    LengthOverflow,
    #[error("CBOR integer cannot be represented")]
    IntegerOverflow,
    #[error("expected CBOR major type {expected}, found {actual}")]
    Type { expected: u8, actual: u8 },
    #[error("unsupported CBOR construct: {0}")]
    Unsupported(&'static str),
    #[error("non-canonical CBOR: {0}")]
    NonCanonical(&'static str),
    #[error("invalid UTF-8 text string")]
    InvalidUtf8,
    #[error("schema violation: {0}")]
    Schema(&'static str),
    #[error("decode limit exceeded: {0}")]
    Limit(&'static str),
    #[error("envelope error: {0}")]
    Envelope(&'static str),
}

pub fn decode_canonical_value(bytes: &[u8], limits: &DecodeLimits) -> Result<Value, DecodeError> {
    if bytes.len() as u64 > limits.max_payload_bytes {
        return Err(DecodeError::Limit("payload bytes"));
    }
    let mut reader = CborReader::new(bytes);
    let mut nodes = 0_u64;
    let value = crate::value::decode_value(&mut reader, limits, 1, &mut nodes)?;
    reader.finish()?;
    let canonical = crate::codec::encode_canonical_value(&value);
    if canonical != bytes {
        return Err(DecodeError::NonCanonical("round-trip mismatch"));
    }
    Ok(value)
}
