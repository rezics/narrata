//! Content references (ADR 0013 §2). Narrata stores these instead of body text, option
//! labels, titles or media; a host-side content provider resolves them.
//!
//! The canonical CBOR form below is the identity encoding. The optional `serde` form
//! (`{"provider": .., "key": ..}`) is for JSON exchange only and never feeds a digest.

#[cfg(feature = "serde")]
mod serde_impl;
#[cfg(test)]
mod tests;

use std::fmt;

use thiserror::Error;

use crate::codec::{CborReader, CborWriter, DecodeError, DecodeLimits, decode_checked};

pub const MAX_PROVIDER_BYTES: usize = 32;
pub const MAX_KEY_BYTES: usize = 256;
pub const MAX_ANCHOR_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ContentError {
    #[error("content provider must be 1..=32 bytes of [a-z0-9-]")]
    Provider,
    #[error("content key must be 1..=256 bytes of UTF-8 without control characters")]
    Key,
    #[error("anchor must be 1..=128 bytes of printable ASCII")]
    Anchor,
}

macro_rules! content_text {
    ($(#[$doc:meta])* $name:ident, $valid:path, $error:expr) => {
        $(#[$doc])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ContentError> {
                let value = value.into();
                if $valid(&value) { Ok(Self(value)) } else { Err($error) }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl std::str::FromStr for $name {
            type Err = ContentError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = ContentError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

content_text!(
    /// A registered content provider such as `local` or `rezics`.
    ProviderId,
    valid_provider,
    ContentError::Provider
);
content_text!(
    /// Opaque to Narrata: compared byte for byte, never parsed.
    ContentKey,
    valid_key,
    ContentError::Key
);
content_text!(
    /// A stable block ID inside one content unit; translations keep the original's anchors.
    AnchorId,
    valid_anchor,
    ContentError::Anchor
);

fn valid_provider(value: &str) -> bool {
    (1..=MAX_PROVIDER_BYTES).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_key(value: &str) -> bool {
    (1..=MAX_KEY_BYTES).contains(&value.len()) && !value.chars().any(char::is_control)
}

fn valid_anchor(value: &str) -> bool {
    (1..=MAX_ANCHOR_BYTES).contains(&value.len())
        && value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContentRef {
    pub provider: ProviderId,
    pub key: ContentKey,
}

/// The blocks of `unit` from `first` (its start when omitted) through `last` (its end when
/// omitted), both ends included, in document order.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Segment {
    pub unit: ContentRef,
    pub first: Option<AnchorId>,
    pub last: Option<AnchorId>,
}

impl ContentRef {
    pub fn new(provider: &str, key: &str) -> Result<Self, ContentError> {
        Ok(Self {
            provider: ProviderId::new(provider)?,
            key: ContentKey::new(key)?,
        })
    }

    /// Canonical form: `{0: provider, 1: key}`.
    pub fn encode(&self, writer: &mut CborWriter) {
        writer.map(2);
        writer.unsigned(0);
        writer.text(self.provider.as_str());
        writer.unsigned(1);
        writer.text(self.key.as_str());
    }

    pub fn decode(reader: &mut CborReader<'_>) -> Result<Self, DecodeError> {
        let mut provider = None;
        let mut key = None;
        let count = reader.map_len()?;
        if count != 2 {
            return Err(DecodeError::Schema("content reference fields"));
        }
        let mut previous = None;
        for _ in 0..count {
            match next_key(reader, &mut previous)? {
                0 => provider = Some(read_text(reader, MAX_PROVIDER_BYTES, ProviderId::new)?),
                1 => key = Some(read_text(reader, MAX_KEY_BYTES, ContentKey::new)?),
                _ => return Err(DecodeError::Schema("unknown content reference field")),
            }
        }
        Ok(Self {
            provider: provider.ok_or(DecodeError::Schema("content reference provider"))?,
            key: key.ok_or(DecodeError::Schema("content reference key"))?,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut writer = CborWriter::new();
        self.encode(&mut writer);
        writer.into_bytes()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        decode_checked(bytes, &limits(), Self::decode, Self::to_bytes)
    }
}

impl Segment {
    /// The whole unit.
    pub fn unit(unit: ContentRef) -> Self {
        Self {
            unit,
            first: None,
            last: None,
        }
    }

    /// Canonical form: `{0: unit, 1?: first, 2?: last}`; absent anchors are omitted.
    pub fn encode(&self, writer: &mut CborWriter) {
        writer.map(1 + u64::from(self.first.is_some()) + u64::from(self.last.is_some()));
        writer.unsigned(0);
        self.unit.encode(writer);
        if let Some(first) = &self.first {
            writer.unsigned(1);
            writer.text(first.as_str());
        }
        if let Some(last) = &self.last {
            writer.unsigned(2);
            writer.text(last.as_str());
        }
    }

    pub fn decode(reader: &mut CborReader<'_>) -> Result<Self, DecodeError> {
        let mut unit = None;
        let mut first = None;
        let mut last = None;
        let count = reader.map_len()?;
        if !(1..=3).contains(&count) {
            return Err(DecodeError::Schema("segment fields"));
        }
        let mut previous = None;
        for _ in 0..count {
            match next_key(reader, &mut previous)? {
                0 => unit = Some(ContentRef::decode(reader)?),
                1 => first = Some(read_text(reader, MAX_ANCHOR_BYTES, AnchorId::new)?),
                2 => last = Some(read_text(reader, MAX_ANCHOR_BYTES, AnchorId::new)?),
                _ => return Err(DecodeError::Schema("unknown segment field")),
            }
        }
        Ok(Self {
            unit: unit.ok_or(DecodeError::Schema("segment unit"))?,
            first,
            last,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut writer = CborWriter::new();
        self.encode(&mut writer);
        writer.into_bytes()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        decode_checked(bytes, &limits(), Self::decode, Self::to_bytes)
    }
}

fn limits() -> DecodeLimits {
    DecodeLimits {
        max_payload_bytes: 4 * 1024,
        max_depth: 4,
        max_string_bytes: MAX_KEY_BYTES as u64,
        max_collection_items: 3,
        max_total_items: 16,
    }
}

fn next_key(reader: &mut CborReader<'_>, previous: &mut Option<u64>) -> Result<u64, DecodeError> {
    let key = reader.unsigned()?;
    if previous.is_some_and(|value| key <= value) {
        return Err(DecodeError::NonCanonical("map key order"));
    }
    *previous = Some(key);
    Ok(key)
}

fn read_text<T>(
    reader: &mut CborReader<'_>,
    max: usize,
    build: impl FnOnce(String) -> Result<T, ContentError>,
) -> Result<T, DecodeError> {
    let text = reader.text(max as u64)?;
    build(text.to_owned()).map_err(|error| {
        DecodeError::Schema(match error {
            ContentError::Provider => "content provider",
            ContentError::Key => "content key",
            ContentError::Anchor => "content anchor",
        })
    })
}
