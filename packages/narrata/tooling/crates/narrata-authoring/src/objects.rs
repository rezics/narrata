//! Identities and strict JSON boundaries shared by the new companion formats.

use narrata_nodes::{Error, Result, canonical_json};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Digest(pub(crate) [u8; 32]);

impl Digest {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
    pub(crate) fn json(domain: &str, bytes: &[u8]) -> Self {
        Self(narrata_kernel::codec::digest_bytes(domain, 1, bytes))
    }
}
impl From<Digest> for String {
    fn from(id: Digest) -> Self {
        hex::encode(id.0)
    }
}
impl std::fmt::Display for Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}
impl TryFrom<String> for Digest {
    type Error = &'static str;
    fn try_from(text: String) -> std::result::Result<Self, Self::Error> {
        if text.len() != 64
            || !text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("expected a 32-byte lowercase hexadecimal digest");
        }
        let mut bytes = [0; 32];
        hex::decode_to_slice(text, &mut bytes).map_err(|_| "invalid digest")?;
        Ok(Self(bytes))
    }
}
impl JsonSchema for Digest {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Digest".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type":"string", "pattern":"^[0-9a-f]{64}$"})
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct Version1;
impl From<Version1> for u16 {
    fn from(_: Version1) -> Self {
        1
    }
}
impl TryFrom<u16> for Version1 {
    type Error = &'static str;
    fn try_from(value: u16) -> std::result::Result<Self, Self::Error> {
        if value == 1 {
            Ok(Self)
        } else {
            Err("unsupported format version")
        }
    }
}
impl JsonSchema for Version1 {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Version1".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"const":1, "type":"integer"})
    }
}

pub(crate) fn error(message: impl std::fmt::Display) -> Error {
    Error::new("publication", "$", message.to_string())
}

pub(crate) fn decode_json<T: serde::de::DeserializeOwned + Serialize>(
    bytes: &[u8],
    limit: usize,
) -> Result<T> {
    let text = std::str::from_utf8(bytes).map_err(error)?;
    let value = narrata_nodes::parse_json_limited(text, limit)?;
    if canonical_json(&value)? != bytes {
        return Err(error("companion object JSON must be canonical"));
    }
    Ok(value)
}
