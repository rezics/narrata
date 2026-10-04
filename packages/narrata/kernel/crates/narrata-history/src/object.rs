use std::sync::Arc;

use narrata_kernel::codec::{EnvelopeLimits, decode_envelope, encode_envelope, inspect_envelope};
use thiserror::Error;

use crate::{HistoryError, ObjectId, object_id};

/// Bytes of the ADR 0003 envelope header that precede the payload.
const HEADER_LENGTH: usize = 56;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ObjectError {
    #[error("object envelope is invalid: {0}")]
    Envelope(String),
    #[error("object exceeds configured byte limit")]
    Limit,
}

/// An immutable object whose envelope was checked and whose identity was recomputed from its
/// kind, schema and payload. Nothing else constructs one, so holding an `Object` means its bytes
/// hash to its id; what the payload means is the business of the kind's registrant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Object {
    id: ObjectId,
    kind: u16,
    schema: u16,
    bytes: Arc<[u8]>,
}

impl Object {
    pub fn new(kind: u16, schema: u16, canonical_payload: &[u8]) -> Self {
        Self {
            id: object_id(kind, schema, canonical_payload),
            kind,
            schema,
            bytes: encode_envelope(kind, schema, canonical_payload).into(),
        }
    }

    /// Checks untrusted envelope bytes that must hold `kind` at `schema`.
    pub fn from_bytes(
        bytes: &[u8],
        kind: u16,
        schema: u16,
        max_bytes: u64,
    ) -> Result<Self, ObjectError> {
        if bytes.len() as u64 > max_bytes {
            return Err(ObjectError::Limit);
        }
        let limits = EnvelopeLimits {
            max_envelope_bytes: max_bytes,
            max_payload_bytes: max_bytes,
        };
        let envelope = decode_envelope(bytes, kind, schema, &limits)
            .map_err(|error| ObjectError::Envelope(error.to_string()))?;
        Ok(Self {
            id: object_id(kind, schema, envelope.payload),
            kind,
            schema,
            bytes: Arc::from(bytes),
        })
    }

    /// Rebuilds an object read back under `id` and refuses bytes that do not hash to it: storage
    /// is outside the trust boundary (ADR 0014).
    pub fn verify(id: ObjectId, bytes: Arc<[u8]>) -> Result<Self, HistoryError> {
        let length = bytes.len() as u64;
        let limits = EnvelopeLimits {
            max_envelope_bytes: length,
            max_payload_bytes: length,
        };
        let envelope = inspect_envelope(&bytes, &limits)
            .map_err(|error| HistoryError::Corrupt(id, error.to_string()))?;
        if object_id(envelope.kind, envelope.schema_version, envelope.payload) != id {
            return Err(HistoryError::Corrupt(id, "ObjectId mismatch".to_owned()));
        }
        let (kind, schema) = (envelope.kind, envelope.schema_version);
        Ok(Self {
            id,
            kind,
            schema,
            bytes,
        })
    }

    pub const fn id(&self) -> ObjectId {
        self.id
    }

    pub const fn kind(&self) -> u16 {
        self.kind
    }

    pub const fn schema(&self) -> u16 {
        self.schema
    }

    /// The whole envelope, as stored and as hashed into bundles.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn shared_bytes(&self) -> Arc<[u8]> {
        Arc::clone(&self.bytes)
    }

    pub fn payload(&self) -> &[u8] {
        self.bytes.get(HEADER_LENGTH..).unwrap_or_default()
    }
}

/// What a manifest records about one object of a bundle closure.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Descriptor {
    pub id: ObjectId,
    pub kind: u16,
    pub schema: u16,
    pub bytes: u64,
}

impl From<&Object> for Descriptor {
    fn from(object: &Object) -> Self {
        Self {
            id: object.id(),
            kind: object.kind(),
            schema: object.schema(),
            bytes: object.bytes().len() as u64,
        }
    }
}
