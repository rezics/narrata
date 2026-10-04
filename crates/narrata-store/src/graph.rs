//! The domain object graph: which objects an object refers to.
//!
//! Write validation, GC marking, bundle closures and the integrity scan all decode references
//! here. A Commit names its Program by artifact, not by object, so callers resolve
//! [`Reference::Program`] in their own context: the engine through its Program index, a bundle
//! through the objects it carries.

use narrata_core::{ObjectId, ProgramArtifactId, codec::ObjectKind};

use crate::{
    CheckedObject, CheckpointBundleManifestV1, CommitCauseV1, CommitV1, CompoundSaveManifestV1,
    HostTimelineManifestV1, StoreError, TimelineArchiveManifestV1, TimelineCatalogEventV1,
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Reference {
    Object {
        id: ObjectId,
        /// The kind the referring object requires, when its schema names one.
        kind: Option<ObjectKind>,
        /// Manifest descriptors keep their objects alive but are not part of a bundle's
        /// closure, which the manifest itself describes.
        descriptor: bool,
    },
    Program(ProgramArtifactId),
}

impl Reference {
    const fn object(id: ObjectId, kind: ObjectKind) -> Self {
        Self::Object {
            id,
            kind: Some(kind),
            descriptor: false,
        }
    }

    const fn untyped(id: ObjectId) -> Self {
        Self::Object {
            id,
            kind: None,
            descriptor: false,
        }
    }
}

/// Kinds whose objects never refer to other objects.
pub(crate) const fn is_leaf(kind: ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::Program
            | ObjectKind::Snapshot
            | ObjectKind::Receipt
            | ObjectKind::Value
            | ObjectKind::EffectResponse
    )
}

pub(crate) fn references(object: &CheckedObject) -> Result<Vec<Reference>, StoreError> {
    let corrupt =
        |error: &dyn std::fmt::Display| StoreError::Corrupt(object.id(), error.to_string());
    let id = |bytes: &[u8; 32]| ObjectId::from_bytes(*bytes);
    let mut values = Vec::new();
    match object.kind() {
        ObjectKind::Commit => {
            let commit = CommitV1::decode(object.payload()).map_err(|error| corrupt(&error))?;
            if let Some(parent) = commit.parent {
                values.push(Reference::object(id(parent.as_bytes()), ObjectKind::Commit));
            }
            values.push(Reference::object(
                id(commit.snapshot.as_bytes()),
                ObjectKind::Snapshot,
            ));
            if let CommitCauseV1::RuntimeTransition(receipt) = commit.cause {
                values.push(Reference::object(
                    id(receipt.as_bytes()),
                    ObjectKind::Receipt,
                ));
            }
            values.push(Reference::Program(commit.program));
        }
        ObjectKind::TimelineCatalogEvent => {
            let event = TimelineCatalogEventV1::decode(object.payload())
                .map_err(|error| corrupt(&error))?;
            if let Some(previous) = event.previous {
                values.push(Reference::object(
                    id(previous.as_bytes()),
                    ObjectKind::TimelineCatalogEvent,
                ));
            }
            for commit in event.referenced_commits() {
                values.push(Reference::object(id(commit.as_bytes()), ObjectKind::Commit));
            }
        }
        ObjectKind::CheckpointBundleManifest => {
            let manifest = CheckpointBundleManifestV1::decode(object.payload())
                .map_err(|error| corrupt(&error))?;
            values.push(Reference::object(
                id(manifest.root.as_bytes()),
                ObjectKind::Commit,
            ));
            values.extend(manifest.objects.iter().map(|descriptor| Reference::Object {
                id: descriptor.id,
                kind: Some(descriptor.kind),
                descriptor: true,
            }));
            values.extend(manifest.optional_host_manifest.map(Reference::untyped));
        }
        ObjectKind::TimelineArchiveManifest => {
            let manifest = TimelineArchiveManifestV1::decode(object.payload())
                .map_err(|error| corrupt(&error))?;
            values.push(Reference::object(
                id(manifest.catalog_head.as_bytes()),
                ObjectKind::TimelineCatalogEvent,
            ));
            values.push(Reference::object(
                id(manifest.coverage.baseline().as_bytes()),
                ObjectKind::Commit,
            ));
            values.extend(manifest.objects.iter().map(|descriptor| Reference::Object {
                id: descriptor.id,
                kind: Some(descriptor.kind),
                descriptor: true,
            }));
            values.extend(manifest.host_timeline.map(Reference::untyped));
        }
        ObjectKind::CompoundSaveManifest => {
            let manifest = CompoundSaveManifestV1::decode(object.payload())
                .map_err(|error| corrupt(&error))?;
            values.push(Reference::object(
                id(manifest.narrative.as_bytes()),
                ObjectKind::Commit,
            ));
        }
        ObjectKind::HostTimelineManifest => {
            let manifest = HostTimelineManifestV1::decode(object.payload())
                .map_err(|error| corrupt(&error))?;
            for entry in &manifest.entries {
                values.push(Reference::object(
                    id(entry.narrative.as_bytes()),
                    ObjectKind::Commit,
                ));
            }
        }
        ObjectKind::Program
        | ObjectKind::Snapshot
        | ObjectKind::Receipt
        | ObjectKind::Value
        | ObjectKind::EffectResponse => {}
    }
    values.sort_unstable();
    values.dedup();
    Ok(values)
}
