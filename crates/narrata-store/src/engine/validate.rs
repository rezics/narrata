//! Write-time validation (ADR 0014): every object a batch adds is checked against the objects it
//! refers to, read from the batch or the store. Objects already stored passed the same check, so
//! every stored object's references are stored too, and no write reads further than one step.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use narrata_core::{
    CheckedProgram, CommitId, ExecutionId, ObjectId, ProgramArtifactId, TimelineCatalogEventId,
    codec::ObjectKind,
    limits::SnapshotLoadLimits,
    snapshot::{restore_snapshot, state_digest},
};
use narrata_storage::{Expect, StorageBackend};

use super::{
    ProgramCache, load_objects,
    write::{Op, Tag},
};
use crate::{
    CATALOG_EVENT_SCHEMA_V1, CHECKPOINT_MANIFEST_SCHEMA_V1, COMPOUND_SAVE_MANIFEST_SCHEMA_V1,
    CheckedObject, CheckpointBundleManifestV1, CommitCauseV1, CommitV1, CompoundSaveManifestV1,
    EFFECT_RESPONSE_SCHEMA_V1, HOST_TIMELINE_MANIFEST_SCHEMA_V1, HostTimelineManifestV1,
    ObjectDescriptor, RecordedEffectResponseV1, STORED_RECEIPT_SCHEMA_V1, StoreError,
    TIMELINE_ARCHIVE_MANIFEST_SCHEMA_V1, TimelineArchiveManifestV1, TimelineCatalogEventV1,
    TimelineOperationId, TransitionReceiptV1,
    commit::COMMIT_SCHEMA_V1,
    graph::{Reference, references},
    layout,
};

type OperationRecord = (Option<TimelineCatalogEventId>, TimelineCatalogEventId);

/// The objects a write can see: its own batch first, then the store.
pub(super) struct View<'a, B> {
    backend: &'a B,
    read_items: u32,
    programs: &'a mut ProgramCache,
    staged: &'a BTreeMap<ObjectId, CheckedObject>,
    staged_programs: BTreeMap<ProgramArtifactId, (ObjectId, Arc<CheckedProgram>)>,
    fetched: BTreeMap<ObjectId, Option<CheckedObject>>,
    operations: BTreeMap<(ExecutionId, TimelineOperationId), OperationRecord>,
    indexed_programs: BTreeSet<ProgramArtifactId>,
}

fn oid(bytes: &[u8; 32]) -> ObjectId {
    ObjectId::from_bytes(*bytes)
}

fn corrupt(id: ObjectId, error: impl std::fmt::Display) -> StoreError {
    StoreError::Corrupt(id, error.to_string())
}

impl<'a, B: StorageBackend> View<'a, B> {
    pub(super) fn new(
        backend: &'a B,
        read_items: u32,
        programs: &'a mut ProgramCache,
        staged: &'a BTreeMap<ObjectId, CheckedObject>,
    ) -> Self {
        let mut staged_programs = BTreeMap::new();
        for object in staged.values() {
            if let Some(program) = programs.load(object) {
                staged_programs
                    .entry(program.artifact_id())
                    .or_insert((object.id(), program));
            }
        }
        Self {
            backend,
            read_items,
            programs,
            staged,
            staged_programs,
            fetched: BTreeMap::new(),
            operations: BTreeMap::new(),
            indexed_programs: BTreeSet::new(),
        }
    }

    pub(super) const fn backend(&self) -> &B {
        self.backend
    }

    pub(super) fn staged_program(&self, artifact: ProgramArtifactId) -> Option<ObjectId> {
        self.staged_programs.get(&artifact).map(|(id, _)| *id)
    }

    /// Reads the stored objects among `ids` in as few backend calls as the read limit allows.
    pub(super) fn prefetch(
        &mut self,
        ids: impl IntoIterator<Item = ObjectId>,
    ) -> Result<(), StoreError> {
        let wanted = ids
            .into_iter()
            .filter(|id| !self.staged.contains_key(id) && !self.fetched.contains_key(id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let found = load_objects(self.backend, self.read_items, &wanted)?;
        self.fetched.extend(wanted.into_iter().zip(found));
        Ok(())
    }

    pub(super) fn object(&mut self, id: ObjectId) -> Result<Option<CheckedObject>, StoreError> {
        if let Some(object) = self.staged.get(&id) {
            return Ok(Some(object.clone()));
        }
        self.prefetch([id])?;
        Ok(self.fetched.get(&id).cloned().flatten())
    }

    pub(super) fn require(
        &mut self,
        id: ObjectId,
        kind: Option<ObjectKind>,
    ) -> Result<CheckedObject, StoreError> {
        let object = self.object(id)?.ok_or(StoreError::MissingObject(id))?;
        if kind.is_none_or(|kind| object.kind() == kind) {
            Ok(object)
        } else {
            Err(StoreError::ObjectKind(id))
        }
    }

    /// Resolves a Commit's Program: from the batch, or through the Program index, whose entries
    /// are checked against the loaded artifact because the index is only a hint.
    pub(super) fn program(
        &mut self,
        artifact: ProgramArtifactId,
    ) -> Result<(ObjectId, Arc<CheckedProgram>), StoreError> {
        if let Some((id, program)) = self.staged_programs.get(&artifact) {
            return Ok((*id, Arc::clone(program)));
        }
        let Some(value) = self
            .backend
            .read_key(layout::PROGRAMS, &layout::program_key(artifact))?
        else {
            return Err(StoreError::InvalidGraph("Program Artifact is missing"));
        };
        let id = layout::decode_program(&value.value)?;
        let object = self
            .object(id)?
            .ok_or(StoreError::InvalidGraph("Program Artifact is missing"))?;
        match self.programs.load(&object) {
            Some(program) if program.artifact_id() == artifact => Ok((id, program)),
            _ => Err(StoreError::CorruptStore(format!(
                "Program index entry {} does not hold artifact {artifact}",
                id
            ))),
        }
    }

    fn commit(&mut self, id: ObjectId) -> Result<CommitV1, StoreError> {
        let object = self.require(id, Some(ObjectKind::Commit))?;
        CommitV1::decode(object.payload()).map_err(|error| corrupt(id, error))
    }

    /// Checks one new object and returns the index keys it adds.
    pub(super) fn validate(&mut self, object: &CheckedObject) -> Result<Vec<Op>, StoreError> {
        for reference in references(object)? {
            if let Reference::Object {
                id,
                kind,
                descriptor,
            } = reference
            {
                // Descriptors are compared field by field below, with their own diagnostic.
                self.require(id, if descriptor { None } else { kind })?;
            }
        }
        let id = object.id();
        let schema = |expected: u16| {
            if object.schema() == expected {
                Ok(())
            } else {
                Err(StoreError::ObjectKind(id))
            }
        };
        match object.kind() {
            ObjectKind::Program => return self.index_program(object),
            ObjectKind::Snapshot | ObjectKind::Value => {}
            ObjectKind::Receipt => {
                schema(STORED_RECEIPT_SCHEMA_V1)?;
                TransitionReceiptV1::decode(object.payload())
                    .map_err(|error| corrupt(id, error))?;
            }
            ObjectKind::Commit => return self.validate_commit(object),
            ObjectKind::TimelineCatalogEvent => {
                schema(CATALOG_EVENT_SCHEMA_V1)?;
                let event = TimelineCatalogEventV1::decode(object.payload())
                    .map_err(|error| corrupt(id, error))?;
                return self
                    .index_operation(&event, TimelineCatalogEventId::from_bytes(*id.as_bytes()));
            }
            ObjectKind::CheckpointBundleManifest => {
                schema(CHECKPOINT_MANIFEST_SCHEMA_V1)?;
                let manifest = CheckpointBundleManifestV1::decode(object.payload())
                    .map_err(|error| corrupt(id, error))?;
                self.check_descriptors(&manifest.objects)?;
            }
            ObjectKind::TimelineArchiveManifest => {
                schema(TIMELINE_ARCHIVE_MANIFEST_SCHEMA_V1)?;
                let manifest = TimelineArchiveManifestV1::decode(object.payload())
                    .map_err(|error| corrupt(id, error))?;
                self.check_descriptors(&manifest.objects)?;
            }
            ObjectKind::EffectResponse => {
                schema(EFFECT_RESPONSE_SCHEMA_V1)?;
                RecordedEffectResponseV1::decode(object.payload())
                    .map_err(|error| corrupt(id, error))?;
            }
            ObjectKind::CompoundSaveManifest => {
                schema(COMPOUND_SAVE_MANIFEST_SCHEMA_V1)?;
                let manifest = CompoundSaveManifestV1::decode(object.payload())
                    .map_err(|error| corrupt(id, error))?;
                let commit = self.commit(oid(manifest.narrative.as_bytes()))?;
                if commit.execution != manifest.execution
                    || commit.program != manifest.program
                    || commit.ledger_fence != manifest.ledger_fence.get()
                {
                    return Err(StoreError::InvalidGraph("Compound Save mismatch"));
                }
            }
            ObjectKind::HostTimelineManifest => {
                schema(HOST_TIMELINE_MANIFEST_SCHEMA_V1)?;
                let manifest = HostTimelineManifestV1::decode(object.payload())
                    .map_err(|error| corrupt(id, error))?;
                for entry in &manifest.entries {
                    let commit = self.commit(oid(entry.narrative.as_bytes()))?;
                    if commit.execution != manifest.execution
                        || commit.ledger_fence != entry.ledger_fence.get()
                    {
                        return Err(StoreError::InvalidGraph("Host Timeline mismatch"));
                    }
                }
            }
        }
        Ok(Vec::new())
    }

    fn check_descriptors(&mut self, descriptors: &[ObjectDescriptor]) -> Result<(), StoreError> {
        for descriptor in descriptors {
            let object = self.require(descriptor.id, None)?;
            if ObjectDescriptor::from(&object) != *descriptor {
                return Err(StoreError::InvalidGraph("manifest descriptor mismatch"));
            }
        }
        Ok(())
    }

    fn index_program(&mut self, object: &CheckedObject) -> Result<Vec<Op>, StoreError> {
        // Objects that do not load as a Program stay stored but unindexed; no Commit can name
        // them.
        let Some(program) = self.programs.load(object) else {
            return Ok(Vec::new());
        };
        let artifact = program.artifact_id();
        if !self.indexed_programs.insert(artifact) {
            return Ok(Vec::new());
        }
        let key = layout::program_key(artifact);
        if let Some(value) = self.backend.read_key(layout::PROGRAMS, &key)? {
            // The first stored object for an artifact stays its entry.
            layout::decode_program(&value.value)?;
            return Ok(Vec::new());
        }
        Ok(vec![Op::put(
            layout::PROGRAMS,
            key,
            layout::encode_program(object.id()),
            Expect::Absent,
            Tag::Replan,
        )])
    }

    fn index_operation(
        &mut self,
        event: &TimelineCatalogEventV1,
        id: TimelineCatalogEventId,
    ) -> Result<Vec<Op>, StoreError> {
        let record = (event.previous, id);
        match self
            .operations
            .insert((event.execution, event.operation), record)
        {
            Some(existing) if existing != record => {
                return Err(StoreError::InvalidGraph("TimelineOperationId conflict"));
            }
            Some(_) => return Ok(Vec::new()),
            None => {}
        }
        let key = layout::catalog_operation_key(event.execution, event.operation);
        match self.backend.read_key(layout::CATALOG_OPERATIONS, &key)? {
            Some(value) if layout::decode_catalog_operation(&value.value)? == record => {
                Ok(Vec::new())
            }
            Some(_) => Err(StoreError::InvalidGraph("TimelineOperationId conflict")),
            None => Ok(vec![Op::put(
                layout::CATALOG_OPERATIONS,
                key,
                layout::encode_catalog_operation(event.previous, id),
                Expect::Absent,
                Tag::Replan,
            )]),
        }
    }

    fn validate_commit(&mut self, object: &CheckedObject) -> Result<Vec<Op>, StoreError> {
        let id = object.id();
        if object.schema() != COMMIT_SCHEMA_V1 {
            return Err(StoreError::ObjectKind(id));
        }
        let commit = CommitV1::decode(object.payload()).map_err(|error| corrupt(id, error))?;
        let (_, program) = self.program(commit.program)?;
        let snapshot_object =
            self.require(oid(commit.snapshot.as_bytes()), Some(ObjectKind::Snapshot))?;
        let limits = SnapshotLoadLimits::default();
        let state = restore_snapshot(snapshot_object.bytes(), &program, &limits)
            .map_err(|error| corrupt(snapshot_object.id(), error))?;
        if state.execution_id != commit.execution || state.turn != commit.turn {
            return Err(StoreError::InvalidGraph("Commit/Snapshot mismatch"));
        }
        match (commit.parent, commit.cause) {
            (None, CommitCauseV1::Genesis) if commit.turn.0 == 0 && commit.ledger_fence == 0 => {}
            (Some(parent_id), CommitCauseV1::RuntimeTransition(receipt_id)) => {
                let parent = self.commit(oid(parent_id.as_bytes()))?;
                if parent.execution != commit.execution
                    || parent.program != commit.program
                    || parent.turn.0.checked_add(1) != Some(commit.turn.0)
                    || commit.ledger_fence < parent.ledger_fence
                {
                    return Err(StoreError::InvalidGraph("Commit parent mismatch"));
                }
                let receipt_object =
                    self.require(oid(receipt_id.as_bytes()), Some(ObjectKind::Receipt))?;
                let receipt = TransitionReceiptV1::decode(receipt_object.payload())
                    .map_err(|error| corrupt(receipt_object.id(), error))?;
                if receipt.execution != commit.execution
                    || receipt.program != commit.program
                    || receipt.parent != parent_id
                    || receipt.next_snapshot != commit.snapshot
                    || receipt.next_state != state_digest(&state)
                {
                    return Err(StoreError::InvalidGraph("Commit/Receipt mismatch"));
                }
                let parent_snapshot =
                    self.require(oid(parent.snapshot.as_bytes()), Some(ObjectKind::Snapshot))?;
                let parent_state = restore_snapshot(parent_snapshot.bytes(), &program, &limits)
                    .map_err(|error| corrupt(parent_snapshot.id(), error))?;
                if receipt.parent_state != state_digest(&parent_state) {
                    return Err(StoreError::InvalidGraph("Receipt parent state mismatch"));
                }
                if state
                    .pending_effect()
                    .is_some_and(|pending| pending.origin_parent_commit != parent_id)
                {
                    return Err(StoreError::InvalidGraph(
                        "pending Effect parent Commit mismatch",
                    ));
                }
            }
            (Some(parent_id), CommitCauseV1::Migration(_)) => {
                let parent = self.commit(oid(parent_id.as_bytes()))?;
                if parent.execution != commit.execution
                    || parent.turn != commit.turn
                    || commit.ledger_fence != parent.ledger_fence
                    || parent.program == commit.program
                {
                    return Err(StoreError::InvalidGraph("migration Commit parent mismatch"));
                }
            }
            _ => return Err(StoreError::InvalidGraph("Commit cause/parent shape")),
        }
        let commit_id = CommitId::from_bytes(*id.as_bytes());
        let mut index = vec![Op::put(
            layout::COMMITS,
            layout::commit_key(commit.execution, commit.turn.0, commit_id),
            layout::encode_marker(),
            Expect::Any,
            Tag::Index,
        )];
        if let Some(parent) = commit.parent {
            index.push(Op::put(
                layout::CHILDREN,
                layout::child_key(parent, commit_id),
                layout::encode_marker(),
                Expect::Any,
                Tag::Index,
            ));
        }
        Ok(index)
    }
}
