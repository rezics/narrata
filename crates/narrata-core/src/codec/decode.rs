use crate::{limits::DecodeLimits, value::Value};

use super::{CborReader, DecodeError};

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
