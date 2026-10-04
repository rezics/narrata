use super::{CborReader, DecodeError};

/// Bounds the structural preflight before a schema decoder is invoked.
/// Depth counts the root as one; item count includes map keys and containers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeLimits {
    pub max_payload_bytes: u64,
    pub max_depth: u32,
    pub max_string_bytes: u64,
    pub max_collection_items: u64,
    pub max_total_items: u64,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_payload_bytes: 16 * 1024 * 1024,
            max_depth: 128,
            max_string_bytes: 1024 * 1024,
            max_collection_items: 1_000_000,
            max_total_items: 2_000_000,
        }
    }
}

/// Checks the ADR 0003 structure and budgets, decodes one schema value, then
/// requires its canonical encoding to match the input exactly. The schema
/// decoder remains responsible for unknown fields and domain invariants.
pub fn decode_checked<'a, T>(
    bytes: &'a [u8],
    limits: &DecodeLimits,
    decode: impl FnOnce(&mut CborReader<'a>) -> Result<T, DecodeError>,
    encode: impl FnOnce(&T) -> Vec<u8>,
) -> Result<T, DecodeError> {
    validate_structure(bytes, limits)?;
    let mut reader = CborReader::new(bytes);
    let value = decode(&mut reader)?;
    reader.finish()?;
    if encode(&value) != bytes {
        return Err(DecodeError::NonCanonical("round-trip mismatch"));
    }
    Ok(value)
}

struct Container {
    remaining: u64,
    is_map: bool,
    last_key: Option<u64>,
}

fn validate_structure(bytes: &[u8], limits: &DecodeLimits) -> Result<(), DecodeError> {
    if bytes.len() as u64 > limits.max_payload_bytes {
        return Err(DecodeError::Limit("payload bytes"));
    }
    let mut reader = CborReader::new(bytes);
    let mut containers: Vec<Container> = Vec::new();
    let mut items = 0_u64;
    loop {
        if containers.len() as u64 >= u64::from(limits.max_depth) {
            return Err(DecodeError::Limit("CBOR depth"));
        }
        if items >= limits.max_total_items {
            return Err(DecodeError::Limit("CBOR items"));
        }
        items += 1;
        let is_key = containers
            .last()
            .is_some_and(|parent| parent.is_map && parent.remaining % 2 == 0);
        let child = if is_key {
            let key = reader.unsigned()?;
            if let Some(parent) = containers.last_mut() {
                if parent.last_key.is_some_and(|previous| key <= previous) {
                    return Err(DecodeError::NonCanonical("map key order"));
                }
                parent.last_key = Some(key);
            }
            None
        } else {
            reader.structural_item(limits.max_string_bytes, limits.max_collection_items)?
        };
        if let Some(parent) = containers.last_mut() {
            parent.remaining -= 1;
        }
        if let Some((remaining, is_map)) = child
            && remaining > 0
        {
            containers.push(Container {
                remaining,
                is_map,
                last_key: None,
            });
        }
        while containers
            .last()
            .is_some_and(|parent| parent.remaining == 0)
        {
            containers.pop();
        }
        if containers.is_empty() {
            return reader.finish();
        }
    }
}
