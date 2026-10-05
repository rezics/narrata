//! The Stage 1–5 kinds as one registrant of the history engine (ADR 0015): their edges, write
//! checks, index keys and the roots kept by catalogs, archives, Compound Saves and the Effect
//! ledger.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, PoisonError},
};

use narrata_core::{
    CheckedProgram, CommitId, CompoundSaveManifestId, ProgramArtifactId, TimelineArchiveManifestId,
    TimelineCatalogEventId,
    codec::ObjectKind,
    version::{PROGRAM_FORMAT_V0, PROGRAM_FORMAT_V1, SNAPSHOT_SCHEMA_V0, SNAPSHOT_SCHEMA_V1},
};
use narrata_history::{
    HistoryError, KindInfo, Object, ObjectId, Op, Reader, Reference, Registry, Root, Tag, View,
};
use narrata_storage::{Conflict, Expect, KeySpace, StorageBackend};

use super::validate;
use crate::{
    ArchiveConflict, CatalogConflict, CatalogHeadRefValue, CatalogRefKey, CommitV1,
    CompoundSaveConflict, CompoundSaveRefKey, CompoundSaveRefValue, RefRevision, StoreError,
    TimelineArchiveRefKey, TimelineArchiveRefValue, TimelineCatalogEventV1, graph, layout,
    object::history_id,
};

/// Loaded Programs kept per store; Programs are few and large.
const PROGRAM_CACHE: usize = 32;

pub(crate) const PROGRAM: u16 = ObjectKind::Program.code();

/// What a failed precondition on one of this registrant's root keys means.
#[derive(Clone, Debug)]
pub(crate) enum RootKey {
    Catalog {
        key: CatalogRefKey,
        expected: Option<RefRevision>,
        proposed: Option<TimelineCatalogEventId>,
    },
    Archive {
        key: TimelineArchiveRefKey,
        expected: Option<RefRevision>,
        proposed: Option<TimelineArchiveManifestId>,
    },
    CompoundSave {
        key: CompoundSaveRefKey,
        expected: Option<RefRevision>,
        proposed: Option<CompoundSaveManifestId>,
    },
}

pub(crate) type LegacyOp = Op<RootKey>;

/// The Stage 1–5 registrant. Programs decoded from verified object bytes are cached by object
/// identity; an entry stays valid whether or not the object is still stored, because presence
/// is always read from the backend.
/// Program formats this registrant reads: 0 carries reader text, 1 content references
/// (ADR 0018). A Program object of another schema stays unindexed, so no Commit can name it.
pub(crate) const PROGRAM_SCHEMAS: [u16; 2] = [PROGRAM_FORMAT_V0.get(), PROGRAM_FORMAT_V1.get()];
/// Snapshot schemas this registrant accepts; each matches the Program format of the same number.
pub(crate) const SNAPSHOT_SCHEMAS: [u16; 2] = [SNAPSHOT_SCHEMA_V0.get(), SNAPSHOT_SCHEMA_V1.get()];

#[derive(Default)]
pub(crate) struct Legacy {
    programs: Mutex<BTreeMap<ObjectId, Option<Arc<CheckedProgram>>>>,
}

impl Legacy {
    pub(crate) fn program(&self, object: &Object) -> Option<Arc<CheckedProgram>> {
        if object.kind() != PROGRAM || !PROGRAM_SCHEMAS.contains(&object.schema()) {
            return None;
        }
        let mut programs = self.programs.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(program) = programs.get(&object.id()) {
            return program.clone();
        }
        let program = narrata_core::program::load_program(object.bytes(), &Default::default()).ok();
        if programs.len() >= PROGRAM_CACHE {
            programs.clear();
        }
        programs.insert(object.id(), program.clone());
        program
    }
}

const fn kind_name(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Program => "Program",
        ObjectKind::Snapshot => "Snapshot",
        ObjectKind::Receipt => "Receipt",
        ObjectKind::Value => "Value",
        ObjectKind::Commit => "Commit",
        ObjectKind::TimelineCatalogEvent => "Timeline Catalog Event",
        ObjectKind::CheckpointBundleManifest => "Checkpoint Bundle Manifest",
        ObjectKind::TimelineArchiveManifest => "Timeline Archive Manifest",
        ObjectKind::EffectResponse => "Effect Response",
        ObjectKind::CompoundSaveManifest => "Compound Save Manifest",
        ObjectKind::HostTimelineManifest => "Host Timeline Manifest",
    }
}

/// Deletes an index entry if it names the object being deleted.
fn index_deletion<B: StorageBackend>(
    reader: &Reader<'_, B>,
    space: KeySpace,
    key: Vec<u8>,
    names_object: impl FnOnce(&[u8]) -> Result<bool, HistoryError>,
) -> Result<Option<LegacyOp>, HistoryError> {
    Ok(match reader.read_key(space, &key)? {
        Some(value) if names_object(&value.value)? => Some(Op::delete(
            space,
            key,
            Expect::Revision(value.revision),
            Tag::Replan,
        )),
        _ => None,
    })
}

impl Registry for Legacy {
    type Error = StoreError;
    type Tag = RootKey;

    fn kind(&self, code: u16) -> Option<KindInfo> {
        let kind = ObjectKind::from_code(code)?;
        (kind != ObjectKind::CheckpointBundleManifest).then_some(KindInfo {
            name: kind_name(kind),
            leaf: graph::is_leaf(kind),
            commit: kind == ObjectKind::Commit,
        })
    }

    fn spaces(&self) -> &[KeySpace] {
        &layout::OWN_SPACES
    }

    fn references(&self, object: &Object) -> Result<Vec<Reference>, HistoryError> {
        graph::references(object)
    }

    fn name(&self, object: &Object) -> Option<[u8; 32]> {
        self.program(object)
            .map(|program| *program.artifact_id().as_bytes())
    }

    fn resolve<B: StorageBackend>(
        &self,
        reader: &Reader<'_, B>,
        kind: u16,
        name: &[u8; 32],
    ) -> Result<Option<ObjectId>, HistoryError> {
        if kind != PROGRAM {
            return Ok(None);
        }
        let key = layout::program_key(ProgramArtifactId::from_bytes(*name));
        reader
            .read_key(layout::PROGRAMS, &key)?
            .map(|value| layout::decode_program(&value.value))
            .transpose()
    }

    fn validate<B: StorageBackend>(
        &self,
        object: &Object,
        view: &mut View<'_, B, Self>,
    ) -> Result<Vec<LegacyOp>, StoreError> {
        validate::validate(self, object, view)
    }

    fn unindex<B: StorageBackend>(
        &self,
        object: &Object,
        reader: &Reader<'_, B>,
    ) -> Result<Vec<LegacyOp>, HistoryError> {
        let id = object.id();
        let mut ops = Vec::new();
        match ObjectKind::from_code(object.kind()) {
            Some(ObjectKind::Commit) => {
                if let Ok(commit) = CommitV1::decode(object.payload()) {
                    let commit_id = CommitId::from_bytes(*id.as_bytes());
                    ops.push(Op::delete(
                        layout::COMMITS,
                        layout::commit_key(commit.execution, commit.turn.0, commit_id),
                        Expect::Any,
                        Tag::Index,
                    ));
                    if let Some(parent) = commit.parent {
                        ops.push(Op::delete(
                            layout::CHILDREN,
                            narrata_history::layout::child_key(history_id(parent.as_bytes()), id),
                            Expect::Any,
                            Tag::Index,
                        ));
                    }
                }
            }
            Some(ObjectKind::Program) => {
                if let Some(program) = self.program(object) {
                    ops.extend(index_deletion(
                        reader,
                        layout::PROGRAMS,
                        layout::program_key(program.artifact_id()),
                        |value| Ok(layout::decode_program(value)? == id),
                    )?);
                }
            }
            Some(ObjectKind::TimelineCatalogEvent) => {
                if let Ok(event) = TimelineCatalogEventV1::decode(object.payload()) {
                    let event_id = TimelineCatalogEventId::from_bytes(*id.as_bytes());
                    ops.extend(index_deletion(
                        reader,
                        layout::CATALOG_OPERATIONS,
                        layout::catalog_operation_key(event.execution, event.operation),
                        |value| Ok(layout::decode_catalog_operation(value)?.1 == event_id),
                    )?);
                }
            }
            _ => {}
        }
        Ok(ops)
    }

    fn roots<B: StorageBackend>(
        &self,
        reader: &Reader<'_, B>,
        _now: u64,
    ) -> Result<Vec<Root>, HistoryError> {
        let root = |bytes: &[u8; 32], kind: ObjectKind| (history_id(bytes), Some(kind.code()));
        let mut roots = Vec::new();
        for entry in reader.scan_every(layout::CATALOG_HEADS, &[])? {
            let (event, _) = layout::decode_catalog_head(&entry.value)?;
            roots.push(root(event.as_bytes(), ObjectKind::TimelineCatalogEvent));
        }
        for entry in reader.scan_every(layout::ARCHIVES, &[])? {
            let manifest = layout::decode_archive(&entry.value)?;
            roots.push(root(
                manifest.as_bytes(),
                ObjectKind::TimelineArchiveManifest,
            ));
        }
        for entry in reader.scan_every(layout::EFFECTS, &[])? {
            let effect = layout::decode_effect(&entry.key, &entry.value)?;
            roots.push(root(effect.origin_commit.as_bytes(), ObjectKind::Commit));
            if let Some(response) = effect.status.response() {
                roots.push(root(response.as_bytes(), ObjectKind::EffectResponse));
            }
        }
        for entry in reader.scan_every(layout::COMPOUND_SAVES, &[])? {
            let manifest = layout::decode_compound_save(&entry.value)?;
            roots.push(root(manifest.as_bytes(), ObjectKind::CompoundSaveManifest));
        }
        Ok(roots)
    }

    fn conflict(&self, tag: &RootKey, conflict: Conflict) -> StoreError {
        let revision = conflict
            .actual
            .as_ref()
            .map(|value| RefRevision::from_revision(value.revision));
        let value = conflict.actual.as_ref().map(|value| value.value.as_slice());
        let error: Result<StoreError, HistoryError> = match tag {
            RootKey::Catalog {
                key,
                expected,
                proposed,
            } => value
                .map(layout::decode_catalog_head)
                .transpose()
                .map(|head| {
                    CatalogConflict {
                        key: key.storage_key(),
                        expected: *expected,
                        actual: head.zip(revision).map(|((event, coverage), revision)| {
                            CatalogHeadRefValue {
                                revision,
                                event,
                                coverage,
                            }
                        }),
                        proposed: *proposed,
                    }
                    .into()
                }),
            RootKey::Archive {
                key,
                expected,
                proposed,
            } => value
                .map(layout::decode_archive)
                .transpose()
                .map(|manifest| {
                    ArchiveConflict {
                        key: key.storage_key(),
                        expected: *expected,
                        actual: manifest.zip(revision).map(|(manifest, revision)| {
                            TimelineArchiveRefValue { revision, manifest }
                        }),
                        proposed: *proposed,
                    }
                    .into()
                }),
            RootKey::CompoundSave {
                key,
                expected,
                proposed,
            } => value
                .map(layout::decode_compound_save)
                .transpose()
                .map(|manifest| {
                    CompoundSaveConflict {
                        key: key.storage_key(),
                        expected: *expected,
                        actual: manifest.zip(revision).map(|(manifest, revision)| {
                            CompoundSaveRefValue { revision, manifest }
                        }),
                        proposed: *proposed,
                    }
                    .into()
                }),
        };
        error.unwrap_or_else(StoreError::from)
    }
}
