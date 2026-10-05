//! The Stage 1–5 object graph: which objects an object refers to, as the history layer's
//! [`Reference`]s.
//!
//! A Commit names its Program by artifact, not by object; the engine resolves that named edge
//! through the Program index, a bundle through the objects it carries. The checkpoint manifest
//! (kind 7) belongs to the history layer, which decodes its references itself.

use narrata_core::codec::ObjectKind;
use narrata_history::{HistoryError, Object, Reference};

use crate::{
    CommitCauseV1, CommitV1, CompoundSaveManifestV1, HostTimelineManifestV1,
    TimelineArchiveManifestV1, TimelineCatalogEventV1, object::history_id,
};

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

fn object(bytes: &[u8; 32], kind: ObjectKind) -> Reference {
    Reference::object(history_id(bytes), kind.code())
}

fn descriptors(descriptors: &[crate::ObjectDescriptor]) -> impl Iterator<Item = Reference> + '_ {
    descriptors.iter().map(|descriptor| {
        Reference::descriptor(history_id(descriptor.id.as_bytes()), descriptor.kind.code())
    })
}

pub(crate) fn references(stored: &Object) -> Result<Vec<Reference>, HistoryError> {
    let corrupt =
        |error: &dyn std::fmt::Display| HistoryError::Corrupt(stored.id(), error.to_string());
    let payload = stored.payload();
    let mut values = Vec::new();
    match ObjectKind::from_code(stored.kind()) {
        Some(ObjectKind::Commit) => {
            let commit = CommitV1::decode(payload).map_err(|error| corrupt(&error))?;
            if let Some(parent) = commit.parent {
                values.push(object(parent.as_bytes(), ObjectKind::Commit));
            }
            values.push(object(commit.snapshot.as_bytes(), ObjectKind::Snapshot));
            if let CommitCauseV1::RuntimeTransition(receipt) = commit.cause {
                values.push(object(receipt.as_bytes(), ObjectKind::Receipt));
            }
            values.push(Reference::Named {
                kind: ObjectKind::Program.code(),
                name: *commit.program.as_bytes(),
            });
        }
        Some(ObjectKind::TimelineCatalogEvent) => {
            let event = TimelineCatalogEventV1::decode(payload).map_err(|error| corrupt(&error))?;
            if let Some(previous) = event.previous {
                values.push(object(
                    previous.as_bytes(),
                    ObjectKind::TimelineCatalogEvent,
                ));
            }
            for commit in event.referenced_commits() {
                values.push(object(commit.as_bytes(), ObjectKind::Commit));
            }
        }
        Some(ObjectKind::TimelineArchiveManifest) => {
            let manifest =
                TimelineArchiveManifestV1::decode(payload).map_err(|error| corrupt(&error))?;
            values.push(object(
                manifest.catalog_head.as_bytes(),
                ObjectKind::TimelineCatalogEvent,
            ));
            values.push(object(
                manifest.coverage.baseline().as_bytes(),
                ObjectKind::Commit,
            ));
            values.extend(descriptors(&manifest.objects));
            values.extend(
                manifest
                    .host_timeline
                    .map(|id| Reference::untyped(history_id(id.as_bytes()))),
            );
        }
        Some(ObjectKind::CompoundSaveManifest) => {
            let manifest =
                CompoundSaveManifestV1::decode(payload).map_err(|error| corrupt(&error))?;
            values.push(object(manifest.narrative.as_bytes(), ObjectKind::Commit));
        }
        Some(ObjectKind::HostTimelineManifest) => {
            let manifest =
                HostTimelineManifestV1::decode(payload).map_err(|error| corrupt(&error))?;
            for entry in &manifest.entries {
                values.push(object(entry.narrative.as_bytes(), ObjectKind::Commit));
            }
        }
        Some(
            ObjectKind::Program
            | ObjectKind::Snapshot
            | ObjectKind::Receipt
            | ObjectKind::Value
            | ObjectKind::EffectResponse
            | ObjectKind::CheckpointBundleManifest,
        )
        | None => {}
    }
    Ok(values)
}
