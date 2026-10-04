use std::sync::Arc;

use narrata_core::{
    CheckpointBundleManifestId, CommitId, CompoundSaveManifestId, HostTimelineManifestId, ObjectId,
    ReceiptId, SnapshotId, TimelineArchiveManifestId, TimelineCatalogEventId,
    codec::{ObjectKind, decode_envelope, encode_envelope},
    limits::DecodeLimits,
};
use thiserror::Error;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ObjectError {
    #[error("object envelope is invalid: {0}")]
    Envelope(String),
    #[error("object exceeds configured byte limit")]
    Limit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckedObject {
    id: ObjectId,
    kind: ObjectKind,
    schema: u16,
    bytes: Arc<[u8]>,
}

impl CheckedObject {
    pub fn new(kind: ObjectKind, schema: u16, canonical_payload: &[u8]) -> Self {
        let id = object_id(kind, schema, canonical_payload);
        let bytes = encode_envelope(kind, schema, canonical_payload).into();
        Self {
            id,
            kind,
            schema,
            bytes,
        }
    }

    pub fn from_bytes(
        bytes: &[u8],
        expected_kind: ObjectKind,
        expected_schema: u16,
        max_bytes: u64,
    ) -> Result<Self, ObjectError> {
        if bytes.len() as u64 > max_bytes {
            return Err(ObjectError::Limit);
        }
        let limits = DecodeLimits {
            max_envelope_bytes: max_bytes,
            max_payload_bytes: max_bytes,
            ..DecodeLimits::default()
        };
        let envelope = decode_envelope(bytes, expected_kind, expected_schema, &limits)
            .map_err(|error| ObjectError::Envelope(error.to_string()))?;
        Ok(Self {
            id: object_id(expected_kind, expected_schema, envelope.payload),
            kind: expected_kind,
            schema: expected_schema,
            bytes: Arc::from(bytes),
        })
    }

    /// For bytes whose envelope was inspected and whose identity was recomputed by the caller.
    pub(crate) const fn from_verified(
        id: ObjectId,
        kind: ObjectKind,
        schema: u16,
        bytes: Arc<[u8]>,
    ) -> Self {
        Self {
            id,
            kind,
            schema,
            bytes,
        }
    }

    pub(crate) fn shared_bytes(&self) -> Arc<[u8]> {
        Arc::clone(&self.bytes)
    }

    pub const fn id(&self) -> ObjectId {
        self.id
    }

    pub const fn kind(&self) -> ObjectKind {
        self.kind
    }

    pub const fn schema(&self) -> u16 {
        self.schema
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn payload(&self) -> &[u8] {
        self.bytes.get(56..).unwrap_or_default()
    }

    pub fn snapshot_id(&self) -> Option<SnapshotId> {
        (self.kind == ObjectKind::Snapshot).then(|| SnapshotId::from_bytes(*self.id.as_bytes()))
    }

    pub fn receipt_id(&self) -> Option<ReceiptId> {
        (self.kind == ObjectKind::Receipt).then(|| ReceiptId::from_bytes(*self.id.as_bytes()))
    }

    pub fn commit_id(&self) -> Option<CommitId> {
        (self.kind == ObjectKind::Commit).then(|| CommitId::from_bytes(*self.id.as_bytes()))
    }

    pub fn catalog_event_id(&self) -> Option<TimelineCatalogEventId> {
        (self.kind == ObjectKind::TimelineCatalogEvent)
            .then(|| TimelineCatalogEventId::from_bytes(*self.id.as_bytes()))
    }

    pub fn checkpoint_manifest_id(&self) -> Option<CheckpointBundleManifestId> {
        (self.kind == ObjectKind::CheckpointBundleManifest)
            .then(|| CheckpointBundleManifestId::from_bytes(*self.id.as_bytes()))
    }

    pub fn timeline_manifest_id(&self) -> Option<TimelineArchiveManifestId> {
        (self.kind == ObjectKind::TimelineArchiveManifest)
            .then(|| TimelineArchiveManifestId::from_bytes(*self.id.as_bytes()))
    }

    pub fn compound_save_manifest_id(&self) -> Option<CompoundSaveManifestId> {
        (self.kind == ObjectKind::CompoundSaveManifest)
            .then(|| CompoundSaveManifestId::from_bytes(*self.id.as_bytes()))
    }

    pub fn host_timeline_manifest_id(&self) -> Option<HostTimelineManifestId> {
        (self.kind == ObjectKind::HostTimelineManifest)
            .then(|| HostTimelineManifestId::from_bytes(*self.id.as_bytes()))
    }
}

pub fn object_id(kind: ObjectKind, schema: u16, canonical_payload: &[u8]) -> ObjectId {
    ObjectId::from_bytes(narrata_core::codec::object_id(
        kind.code(),
        schema,
        canonical_payload,
    ))
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ObjectDescriptor {
    pub id: ObjectId,
    pub kind: ObjectKind,
    pub schema: u16,
    pub bytes: u64,
}

impl From<&CheckedObject> for ObjectDescriptor {
    fn from(object: &CheckedObject) -> Self {
        Self {
            id: object.id(),
            kind: object.kind(),
            schema: object.schema(),
            bytes: object.bytes().len() as u64,
        }
    }
}
