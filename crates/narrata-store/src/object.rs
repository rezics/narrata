use narrata_core::{
    CheckpointBundleManifestId, CommitId, CompoundSaveManifestId, HostTimelineManifestId, ObjectId,
    ReceiptId, SnapshotId, TimelineArchiveManifestId, TimelineCatalogEventId,
    codec::{DecodeError, ObjectKind},
};
pub use narrata_history::ObjectError;
use narrata_history::{Descriptor, Object};

use crate::StoreError;

/// A history [`Object`] of one of the Stage 1–5 kinds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckedObject {
    object: Object,
    kind: ObjectKind,
}

impl CheckedObject {
    pub fn new(kind: ObjectKind, schema: u16, canonical_payload: &[u8]) -> Self {
        Self {
            object: Object::new(kind.code(), schema, canonical_payload),
            kind,
        }
    }

    pub fn from_bytes(
        bytes: &[u8],
        expected_kind: ObjectKind,
        expected_schema: u16,
        max_bytes: u64,
    ) -> Result<Self, ObjectError> {
        Ok(Self {
            object: Object::from_bytes(bytes, expected_kind.code(), expected_schema, max_bytes)?,
            kind: expected_kind,
        })
    }

    /// Narrows a history object to the Stage 1–5 kinds; any other kind reads as corrupt here.
    pub(crate) fn from_object(object: Object) -> Result<Self, StoreError> {
        let kind = ObjectKind::from_code(object.kind()).ok_or_else(|| {
            StoreError::Corrupt(
                id(object.id()),
                DecodeError::Envelope("unknown object kind").to_string(),
            )
        })?;
        Ok(Self { object, kind })
    }

    pub(crate) fn object(&self) -> &Object {
        &self.object
    }

    pub(crate) fn into_object(self) -> Object {
        self.object
    }

    pub const fn id(&self) -> ObjectId {
        id(self.object.id())
    }

    pub const fn kind(&self) -> ObjectKind {
        self.kind
    }

    pub const fn schema(&self) -> u16 {
        self.object.schema()
    }

    pub fn bytes(&self) -> &[u8] {
        self.object.bytes()
    }

    pub fn payload(&self) -> &[u8] {
        self.object.payload()
    }

    pub fn snapshot_id(&self) -> Option<SnapshotId> {
        (self.kind == ObjectKind::Snapshot).then(|| SnapshotId::from_bytes(*self.id().as_bytes()))
    }

    pub fn receipt_id(&self) -> Option<ReceiptId> {
        (self.kind == ObjectKind::Receipt).then(|| ReceiptId::from_bytes(*self.id().as_bytes()))
    }

    pub fn commit_id(&self) -> Option<CommitId> {
        (self.kind == ObjectKind::Commit).then(|| CommitId::from_bytes(*self.id().as_bytes()))
    }

    pub fn catalog_event_id(&self) -> Option<TimelineCatalogEventId> {
        (self.kind == ObjectKind::TimelineCatalogEvent)
            .then(|| TimelineCatalogEventId::from_bytes(*self.id().as_bytes()))
    }

    pub fn checkpoint_manifest_id(&self) -> Option<CheckpointBundleManifestId> {
        (self.kind == ObjectKind::CheckpointBundleManifest)
            .then(|| CheckpointBundleManifestId::from_bytes(*self.id().as_bytes()))
    }

    pub fn timeline_manifest_id(&self) -> Option<TimelineArchiveManifestId> {
        (self.kind == ObjectKind::TimelineArchiveManifest)
            .then(|| TimelineArchiveManifestId::from_bytes(*self.id().as_bytes()))
    }

    pub fn compound_save_manifest_id(&self) -> Option<CompoundSaveManifestId> {
        (self.kind == ObjectKind::CompoundSaveManifest)
            .then(|| CompoundSaveManifestId::from_bytes(*self.id().as_bytes()))
    }

    pub fn host_timeline_manifest_id(&self) -> Option<HostTimelineManifestId> {
        (self.kind == ObjectKind::HostTimelineManifest)
            .then(|| HostTimelineManifestId::from_bytes(*self.id().as_bytes()))
    }
}

pub fn object_id(kind: ObjectKind, schema: u16, canonical_payload: &[u8]) -> ObjectId {
    id(narrata_history::object_id(
        kind.code(),
        schema,
        canonical_payload,
    ))
}

/// The Stage 1–5 identity of a history object.
pub(crate) const fn id(id: narrata_history::ObjectId) -> ObjectId {
    ObjectId::from_bytes(*id.as_bytes())
}

/// The history identity of a Stage 1–5 object, Commit or manifest.
pub(crate) const fn history_id(bytes: &[u8; 32]) -> narrata_history::ObjectId {
    narrata_history::ObjectId::from_bytes(*bytes)
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ObjectDescriptor {
    pub id: ObjectId,
    pub kind: ObjectKind,
    pub schema: u16,
    pub bytes: u64,
}

impl ObjectDescriptor {
    /// Narrows a history descriptor to the Stage 1–5 kinds.
    pub(crate) fn from_descriptor(descriptor: Descriptor) -> Option<Self> {
        Some(Self {
            id: id(descriptor.id),
            kind: ObjectKind::from_code(descriptor.kind)?,
            schema: descriptor.schema,
            bytes: descriptor.bytes,
        })
    }

    pub(crate) const fn descriptor(&self) -> Descriptor {
        Descriptor {
            id: history_id(self.id.as_bytes()),
            kind: self.kind.code(),
            schema: self.schema,
            bytes: self.bytes,
        }
    }
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
