//! The key spaces the history layer owns, and the strict value codecs every registrant uses for
//! its own spaces (ADR 0014, ADR 0015).
//!
//! Keys are raw fixed-width identities and `RefName` bytes; values are canonical CBOR maps.
//! Every decoder is strict: it rejects trailing bytes, unknown shapes and any encoding that does
//! not re-encode to the same bytes, because the backend is outside the trust boundary.

use narrata_kernel::codec::{CborReader, CborWriter, DecodeError};
use narrata_storage::KeySpace;

use crate::{HistoryError, ObjectId, RefKey, RefName, RefNamespace};

/// Version recorded under `meta/layout`.
pub const LAYOUT_VERSION: u64 = 1;

pub const META: KeySpace = KeySpace::new(0);
pub const TOUCH: KeySpace = KeySpace::new(1);
pub const REFS: KeySpace = KeySpace::new(2);
pub const PINS: KeySpace = KeySpace::new(7);
pub const CHILDREN: KeySpace = KeySpace::new(12);
/// `(parent, input) → commit` for commits in the generic format (ADR 0015).
pub const TRANSITIONS: KeySpace = KeySpace::new(14);

/// The spaces of this layer with their names, in space order. Spaces 3–6, 8–11 and 13 belong to
/// the Stage 1–5 registrant.
pub const SPACES: [(KeySpace, &str); 6] = [
    (META, "meta"),
    (TOUCH, "touch"),
    (REFS, "refs"),
    (PINS, "pins"),
    (CHILDREN, "children"),
    (TRANSITIONS, "transitions"),
];

pub const LAYOUT_KEY: &[u8] = b"layout";
/// Bumped by every batch that adds objects or roots; GC deletion batches check it.
pub const GRAPH_KEY: &[u8] = b"graph";
/// Bumped by every GC deletion batch; writers check it.
pub const SWEEP_KEY: &[u8] = b"sweep";

/// Separates names inside a key; [`RefName`]s never contain it.
pub const SEPARATOR: u8 = 0;

pub fn corrupt(what: &str, error: impl std::fmt::Display) -> HistoryError {
    HistoryError::CorruptStore(format!("{what}: {error}"))
}

/// Decodes one stored value and requires it to re-encode to the same bytes.
pub fn decode<'a, T>(
    what: &'static str,
    bytes: &'a [u8],
    read: impl FnOnce(&mut CborReader<'a>) -> Result<T, DecodeError>,
    encode: impl FnOnce(&T) -> Vec<u8>,
) -> Result<T, HistoryError> {
    let mut reader = CborReader::new(bytes);
    let value = read(&mut reader)
        .and_then(|value| reader.finish().map(|()| value))
        .map_err(|error| corrupt(what, error))?;
    if encode(&value) == bytes {
        Ok(value)
    } else {
        Err(corrupt(
            what,
            DecodeError::NonCanonical("round-trip mismatch"),
        ))
    }
}

pub fn expect_map(reader: &mut CborReader<'_>, length: u64) -> Result<(), DecodeError> {
    if reader.map_len()? == length {
        Ok(())
    } else {
        Err(DecodeError::Schema("map field count"))
    }
}

pub fn expect_array(reader: &mut CborReader<'_>, length: u64) -> Result<(), DecodeError> {
    if reader.array_len()? == length {
        Ok(())
    } else {
        Err(DecodeError::Schema("array field count"))
    }
}

pub fn key(reader: &mut CborReader<'_>, expected: u64) -> Result<(), DecodeError> {
    if reader.unsigned()? == expected {
        Ok(())
    } else {
        Err(DecodeError::Schema("map key order"))
    }
}

pub fn fixed<const N: usize>(what: &'static str, bytes: &[u8]) -> Result<[u8; N], HistoryError> {
    bytes
        .try_into()
        .map_err(|_| corrupt(what, "wrong key length"))
}

pub fn split<'a>(
    what: &'static str,
    bytes: &'a [u8],
    at: usize,
) -> Result<(&'a [u8], &'a [u8]), HistoryError> {
    if bytes.len() < at {
        return Err(corrupt(what, "key is truncated"));
    }
    Ok(bytes.split_at(at))
}

pub fn name(what: &'static str, bytes: &[u8]) -> Result<RefName, HistoryError> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|value| RefName::new(value).ok())
        .ok_or_else(|| corrupt(what, "invalid name"))
}

/// Splits `first ‖ 0x00 ‖ second` into two names.
pub fn names(what: &'static str, bytes: &[u8]) -> Result<(RefName, RefName), HistoryError> {
    let position = bytes
        .iter()
        .position(|byte| *byte == SEPARATOR)
        .ok_or_else(|| corrupt(what, "missing separator"))?;
    Ok((
        name(what, &bytes[..position])?,
        name(what, &bytes[position + 1..])?,
    ))
}

pub fn named(first: &[u8], second: &[u8]) -> Vec<u8> {
    let mut key = Vec::with_capacity(first.len() + second.len() + 1);
    key.extend_from_slice(first);
    key.push(SEPARATOR);
    key.extend_from_slice(second);
    key
}

fn empty_map() -> Vec<u8> {
    let mut writer = CborWriter::new();
    writer.map(0);
    writer.into_bytes()
}

/// `{0: value}` with a byte-string value.
pub fn single_bytes(value: &[u8]) -> Vec<u8> {
    let mut writer = CborWriter::new();
    writer.map(1);
    writer.unsigned(0);
    writer.bytes(value);
    writer.into_bytes()
}

pub fn decode_single_id(what: &'static str, bytes: &[u8]) -> Result<[u8; 32], HistoryError> {
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

/// `{0: value}` with an unsigned value.
pub fn single_unsigned(value: u64) -> Vec<u8> {
    let mut writer = CborWriter::new();
    writer.map(1);
    writer.unsigned(0);
    writer.unsigned(value);
    writer.into_bytes()
}

pub fn decode_single_unsigned(what: &'static str, bytes: &[u8]) -> Result<u64, HistoryError> {
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

pub fn encode_layout() -> Vec<u8> {
    single_unsigned(LAYOUT_VERSION)
}

pub fn decode_layout(bytes: &[u8]) -> Result<u64, HistoryError> {
    decode_single_unsigned("layout version", bytes)
}

/// `{}`: the value of keys whose presence or revision is the information.
pub fn encode_marker() -> Vec<u8> {
    empty_map()
}

pub fn decode_marker(what: &'static str, bytes: &[u8]) -> Result<(), HistoryError> {
    decode(
        what,
        bytes,
        |reader| expect_map(reader, 0),
        |()| empty_map(),
    )
}

// touch

pub fn touch_key(object: ObjectId) -> Vec<u8> {
    object.as_bytes().to_vec()
}

pub fn decode_touch_key(bytes: &[u8]) -> Result<ObjectId, HistoryError> {
    fixed::<32>("touch key", bytes).map(ObjectId::from_bytes)
}

pub fn encode_touch(observed_at: u64) -> Vec<u8> {
    single_unsigned(observed_at)
}

pub fn decode_touch(bytes: &[u8]) -> Result<u64, HistoryError> {
    decode_single_unsigned("touch value", bytes)
}

// refs

pub fn ref_key(key: &RefKey) -> Vec<u8> {
    let mut bytes = vec![key.namespace() as u8];
    bytes.extend(named(
        key.owner().as_str().as_bytes(),
        key.name().as_str().as_bytes(),
    ));
    bytes
}

pub fn ref_prefix(namespace: Option<RefNamespace>, owner: Option<&RefName>) -> Vec<u8> {
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

pub fn decode_ref_key(bytes: &[u8]) -> Result<RefKey, HistoryError> {
    let (namespace, rest) = split("Ref key", bytes, 1)?;
    let namespace =
        RefNamespace::from_code(namespace[0]).ok_or_else(|| corrupt("Ref key", "namespace"))?;
    let (owner, name) = names("Ref key", rest)?;
    Ok(RefKey::new(namespace, owner, name))
}

pub fn encode_ref_target(commit: ObjectId) -> Vec<u8> {
    single_bytes(commit.as_bytes())
}

pub fn decode_ref_target(bytes: &[u8]) -> Result<ObjectId, HistoryError> {
    decode_single_id("Ref value", bytes).map(ObjectId::from_bytes)
}

// pins

/// Pin owners are free text, so they may not contain the key separator.
pub fn pin_key(owner: &str, object: ObjectId) -> Result<Vec<u8>, HistoryError> {
    if owner.as_bytes().contains(&SEPARATOR) {
        return Err(HistoryError::InvalidGraph("Pin owner contains NUL"));
    }
    Ok(named(owner.as_bytes(), object.as_bytes()))
}

pub fn decode_pin_key(bytes: &[u8]) -> Result<(String, ObjectId), HistoryError> {
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

pub fn encode_pin(expires_at: Option<u64>) -> Vec<u8> {
    let mut writer = CborWriter::new();
    writer.map(1);
    writer.unsigned(0);
    match expires_at {
        Some(value) => writer.unsigned(value),
        None => writer.null(),
    }
    writer.into_bytes()
}

pub fn decode_pin_value(bytes: &[u8]) -> Result<Option<u64>, HistoryError> {
    decode(
        "Pin value",
        bytes,
        |reader| {
            expect_map(reader, 1)?;
            key(reader, 0)?;
            reader.optional(CborReader::unsigned)
        },
        |value| encode_pin(*value),
    )
}

// children

pub fn child_key(parent: ObjectId, child: ObjectId) -> Vec<u8> {
    [parent.as_bytes().as_slice(), child.as_bytes()].concat()
}

pub fn decode_child_key(bytes: &[u8]) -> Result<(ObjectId, ObjectId), HistoryError> {
    let (parent, child) = split("child key", bytes, 32)?;
    Ok((
        ObjectId::from_bytes(fixed("child key", parent)?),
        ObjectId::from_bytes(fixed("child key", child)?),
    ))
}

// transitions

pub fn transition_key(parent: ObjectId, input: ObjectId) -> Vec<u8> {
    [parent.as_bytes().as_slice(), input.as_bytes()].concat()
}

pub fn encode_transition(commit: ObjectId) -> Vec<u8> {
    single_bytes(commit.as_bytes())
}

pub fn decode_transition(bytes: &[u8]) -> Result<ObjectId, HistoryError> {
    decode_single_id("transition value", bytes).map(ObjectId::from_bytes)
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
        let commit = ObjectId::from_bytes([7; 32]);
        let encoded = encode_ref_target(commit);
        assert_eq!(decode_ref_target(&encoded).ok(), Some(commit));
        let mut trailing = encoded.clone();
        trailing.push(0);
        rejects(decode_ref_target(&trailing));
        // Key 0 written as a one-byte-argument integer is valid CBOR but not canonical.
        let mut noncanonical = vec![0xa1, 0x18, 0x00];
        noncanonical.extend_from_slice(&encoded[2..]);
        rejects(decode_ref_target(&noncanonical));
        rejects(decode_touch(&encode_ref_target(commit)));
        assert_eq!(decode_touch(&encode_touch(42)).ok(), Some(42));
        assert_eq!(decode_layout(&encode_layout()).ok(), Some(LAYOUT_VERSION));
        assert!(decode_marker("graph", &encode_marker()).is_ok());
        rejects(decode_marker("graph", &encode_touch(0)));
        assert_eq!(
            decode_transition(&encode_transition(commit)).ok(),
            Some(commit)
        );
        assert_eq!(decode_pin_value(&encode_pin(Some(3))).ok(), Some(Some(3)));
        assert_eq!(decode_pin_value(&encode_pin(None)).ok(), Some(None));
    }

    #[test]
    fn keys_round_trip_and_keep_tuple_order() {
        let name = |value: &str| RefName::new(value).unwrap();
        let short = RefKey::save(name("a"), name("z"));
        let long = RefKey::save(name("ab"), name("a"));
        assert!(ref_key(&short) < ref_key(&long));
        assert_eq!(decode_ref_key(&ref_key(&long)).ok(), Some(long));
        let object = ObjectId::from_bytes([3; 32]);
        let pin = pin_key("owner", object).unwrap();
        assert_eq!(
            decode_pin_key(&pin).ok(),
            Some(("owner".to_owned(), object))
        );
        assert!(pin_key("a\0b", object).is_err());
        assert!(pin_key("a", object).unwrap() < pin_key("ab", object).unwrap());
        let parent = ObjectId::from_bytes([1; 32]);
        assert_eq!(
            decode_child_key(&child_key(parent, object)).ok(),
            Some((parent, object))
        );
        rejects(decode_ref_key(&[9, b'a', 0, b'b']));
        rejects(decode_ref_key(&[0, b'a', b'b']));
        rejects(decode_child_key(&[0; 63]));
    }
}
