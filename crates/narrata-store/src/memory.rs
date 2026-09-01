use std::collections::{BTreeMap, BTreeSet, VecDeque};

use narrata_core::{
    ExecutionId, InputId, ObjectId, ProgramArtifactId, TimelineCatalogEventId,
    codec::ObjectKind,
    limits::{ProgramLoadLimits, SnapshotLoadLimits},
    program::load_program,
    snapshot::{restore_snapshot, state_digest},
};

use crate::{
    CATALOG_EVENT_SCHEMA_V1, CHECKPOINT_MANIFEST_SCHEMA_V1, CatalogHeadRefValue, CatalogRefKey,
    CheckedObject, CommitCauseV1, CommitOutcome, CommitTransaction, CommitV1, FaultPoint, GcReport,
    InputIdConflict, InputRecord, IntegrityIssue, ManifestError, Pin, PutOutcome, RefKey, RefValue,
    RetentionPolicy, STORED_RECEIPT_SCHEMA_V1, SaveStore, StoreError,
    TIMELINE_ARCHIVE_MANIFEST_SCHEMA_V1, TimelineArchiveManifestV1, TimelineArchiveRefKey,
    TimelineArchiveRefValue, TimelineCatalogEventV1, TransitionReceiptV1,
    commit::COMMIT_SCHEMA_V1,
    manifest::CheckpointBundleManifestV1,
    store::{
        CatalogOperationIndex, CatalogOperationRecord, check_expected_archive,
        check_expected_catalog, check_expected_ref, next_revision,
    },
};

#[derive(Clone, Debug)]
struct StoredObject {
    object: CheckedObject,
    inserted_at: u64,
}

#[derive(Clone, Debug, Default)]
pub struct MemoryStoreState {
    pub objects: Vec<(CheckedObject, u64)>,
    pub refs: Vec<(RefKey, RefValue)>,
    pub catalogs: Vec<(CatalogRefKey, CatalogHeadRefValue)>,
    pub archives: Vec<(TimelineArchiveRefKey, TimelineArchiveRefValue)>,
    pub inputs: Vec<InputRecord>,
    pub pins: Vec<Pin>,
}

#[derive(Clone, Debug, Default)]
pub struct MemoryStore {
    objects: BTreeMap<ObjectId, StoredObject>,
    refs: BTreeMap<RefKey, RefValue>,
    catalogs: BTreeMap<CatalogRefKey, CatalogHeadRefValue>,
    archives: BTreeMap<TimelineArchiveRefKey, TimelineArchiveRefValue>,
    inputs: BTreeMap<(ExecutionId, InputId), InputRecord>,
    pins: BTreeMap<(String, ObjectId), Pin>,
    catalog_operations: CatalogOperationIndex,
    fault: Option<FaultPoint>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inject_fault(&mut self, point: Option<FaultPoint>) {
        self.fault = point;
    }

    pub fn from_state(state: MemoryStoreState) -> Result<Self, StoreError> {
        let mut store = Self::new();
        for (object, inserted_at) in state.objects {
            store.put_object(object, inserted_at)?;
        }
        store.refs = state.refs.into_iter().collect();
        store.catalogs = state.catalogs.into_iter().collect();
        store.archives = state.archives.into_iter().collect();
        store.inputs = state
            .inputs
            .into_iter()
            .map(|value| ((value.execution, value.input), value))
            .collect();
        store.pins = state
            .pins
            .into_iter()
            .map(|value| ((value.owner.clone(), value.object), value))
            .collect();
        store.validate_all()?;
        for value in store.refs.values() {
            store.require_kind(object_id(value.commit.as_bytes()), ObjectKind::Commit)?;
        }
        for value in store.catalogs.values() {
            store.require_kind(
                object_id(value.event.as_bytes()),
                ObjectKind::TimelineCatalogEvent,
            )?;
        }
        for value in store.archives.values() {
            store.require_kind(
                object_id(value.manifest.as_bytes()),
                ObjectKind::TimelineArchiveManifest,
            )?;
        }
        Ok(store)
    }

    pub fn export_state(&self) -> MemoryStoreState {
        MemoryStoreState {
            objects: self
                .objects
                .values()
                .map(|stored| (stored.object.clone(), stored.inserted_at))
                .collect(),
            refs: self
                .refs
                .iter()
                .map(|(key, value)| (key.clone(), *value))
                .collect(),
            catalogs: self
                .catalogs
                .iter()
                .map(|(key, value)| (key.clone(), *value))
                .collect(),
            archives: self
                .archives
                .iter()
                .map(|(key, value)| (key.clone(), *value))
                .collect(),
            inputs: self.inputs.values().copied().collect(),
            pins: self.pins.values().cloned().collect(),
        }
    }

    pub fn put_object(
        &mut self,
        object: CheckedObject,
        observed_at: u64,
    ) -> Result<PutOutcome, StoreError> {
        match self.objects.get(&object.id()) {
            Some(existing) if existing.object.bytes() == object.bytes() => {
                Ok(PutOutcome::AlreadyPresent)
            }
            Some(_) => Err(StoreError::Corrupt(
                object.id(),
                "same ObjectId has different bytes".to_owned(),
            )),
            None => {
                self.objects.insert(
                    object.id(),
                    StoredObject {
                        object,
                        inserted_at: observed_at,
                    },
                );
                Ok(PutOutcome::Inserted)
            }
        }
    }

    fn fail(&self, point: FaultPoint) -> Result<(), StoreError> {
        if self.fault == Some(point) {
            Err(StoreError::InjectedFault(point))
        } else {
            Ok(())
        }
    }

    fn apply(&mut self, transaction: CommitTransaction) -> Result<CommitOutcome, StoreError> {
        let mut outcome = CommitOutcome::default();
        for object in transaction.objects {
            let point = match object.kind() {
                ObjectKind::Snapshot => FaultPoint::SnapshotWrite,
                ObjectKind::Receipt => FaultPoint::ReceiptWrite,
                ObjectKind::Commit => FaultPoint::CommitWrite,
                ObjectKind::TimelineCatalogEvent => FaultPoint::CatalogEventWrite,
                ObjectKind::Program
                | ObjectKind::Value
                | ObjectKind::CheckpointBundleManifest
                | ObjectKind::TimelineArchiveManifest => FaultPoint::EdgeIndexWrite,
            };
            self.fail(point)?;
            if self.put_object(object, transaction.observed_at)? == PutOutcome::Inserted {
                outcome.inserted_objects = outcome.inserted_objects.saturating_add(1);
            }
        }
        self.fail(FaultPoint::EdgeIndexWrite)?;
        self.validate_all()?;

        for input in transaction.inputs {
            match self.inputs.get(&(input.execution, input.input)).copied() {
                Some(existing) if existing == input => {}
                Some(existing) => {
                    return Err(InputIdConflict {
                        existing,
                        proposed: input,
                    }
                    .into());
                }
                None => {
                    self.inputs.insert((input.execution, input.input), input);
                }
            }
        }

        for mutation in transaction.refs {
            self.fail(if transaction.import_transaction {
                FaultPoint::BundleImportRef
            } else {
                FaultPoint::RefCas
            })?;
            let actual = self.refs.get(&mutation.key).copied();
            check_expected_ref(&mutation.key, mutation.expected, actual, mutation.next)?;
            let next = match mutation.next {
                Some(commit) => {
                    self.require_kind(object_id(commit.as_bytes()), ObjectKind::Commit)?;
                    let value = RefValue {
                        revision: next_revision(actual.map(|value| value.revision))?,
                        commit,
                    };
                    self.refs.insert(mutation.key.clone(), value);
                    Some(value)
                }
                None => {
                    self.refs.remove(&mutation.key);
                    None
                }
            };
            outcome.refs.insert(mutation.key, next);
        }

        for mutation in transaction.catalogs {
            self.fail(FaultPoint::CatalogHeadCas)?;
            let actual = self.catalogs.get(&mutation.key).copied();
            check_expected_catalog(&mutation.key, mutation.expected, actual, mutation.next)?;
            let next = match mutation.next {
                Some(event) => {
                    self.require_kind(
                        object_id(event.as_bytes()),
                        ObjectKind::TimelineCatalogEvent,
                    )?;
                    let value = CatalogHeadRefValue {
                        revision: next_revision(actual.map(|value| value.revision))?,
                        event,
                        coverage: mutation.coverage,
                    };
                    self.catalogs.insert(mutation.key.clone(), value);
                    Some(value)
                }
                None => {
                    self.catalogs.remove(&mutation.key);
                    None
                }
            };
            outcome.catalogs.insert(mutation.key, next);
        }

        for mutation in transaction.archives {
            self.fail(FaultPoint::ArchiveRefCas)?;
            let actual = self.archives.get(&mutation.key).copied();
            check_expected_archive(&mutation.key, mutation.expected, actual, mutation.next)?;
            let next = match mutation.next {
                Some(manifest) => {
                    self.require_kind(
                        object_id(manifest.as_bytes()),
                        ObjectKind::TimelineArchiveManifest,
                    )?;
                    let value = TimelineArchiveRefValue {
                        revision: next_revision(actual.map(|value| value.revision))?,
                        manifest,
                    };
                    self.archives.insert(mutation.key.clone(), value);
                    Some(value)
                }
                None => {
                    self.archives.remove(&mutation.key);
                    None
                }
            };
            outcome.archives.insert(mutation.key, next);
        }

        for pin in transaction.pins {
            self.require_object(pin.object)?;
            self.pins.insert((pin.owner.clone(), pin.object), pin);
        }
        for key in transaction.remove_pins {
            self.pins.remove(&key);
        }
        self.fail(FaultPoint::TransactionCommit)?;
        Ok(outcome)
    }

    fn require_object(&self, id: ObjectId) -> Result<&CheckedObject, StoreError> {
        self.objects
            .get(&id)
            .map(|stored| &stored.object)
            .ok_or(StoreError::MissingObject(id))
    }

    fn require_kind(&self, id: ObjectId, kind: ObjectKind) -> Result<&CheckedObject, StoreError> {
        let object = self.require_object(id)?;
        if object.kind() == kind {
            Ok(object)
        } else {
            Err(StoreError::ObjectKind(id))
        }
    }

    fn find_program(
        &self,
        id: ProgramArtifactId,
    ) -> Result<(ObjectId, std::sync::Arc<narrata_core::CheckedProgram>), StoreError> {
        for (object_id, stored) in &self.objects {
            if stored.object.kind() != ObjectKind::Program || stored.object.schema() != 0 {
                continue;
            }
            if let Ok(program) = load_program(stored.object.bytes(), &ProgramLoadLimits::default())
                && program.artifact_id() == id
            {
                return Ok((*object_id, program));
            }
        }
        Err(StoreError::InvalidGraph("Program Artifact is missing"))
    }

    fn validate_all(&mut self) -> Result<(), StoreError> {
        let ids = self.objects.keys().copied().collect::<Vec<_>>();
        let mut operations = CatalogOperationIndex::new();
        for id in ids {
            let object = self.require_object(id)?;
            match object.kind() {
                ObjectKind::Program | ObjectKind::Snapshot | ObjectKind::Value => {}
                ObjectKind::Receipt => {
                    if object.schema() != STORED_RECEIPT_SCHEMA_V1 {
                        return Err(StoreError::ObjectKind(id));
                    }
                    TransitionReceiptV1::decode(object.payload())
                        .map_err(|error| StoreError::Corrupt(id, error.to_string()))?;
                }
                ObjectKind::Commit => self.validate_commit(object)?,
                ObjectKind::TimelineCatalogEvent => {
                    if object.schema() != CATALOG_EVENT_SCHEMA_V1 {
                        return Err(StoreError::ObjectKind(id));
                    }
                    let event = TimelineCatalogEventV1::decode(object.payload())
                        .map_err(|error| StoreError::Corrupt(id, error.to_string()))?;
                    let event_id = TimelineCatalogEventId::from_bytes(*id.as_bytes());
                    if let Some(previous) = event.previous {
                        self.require_kind(
                            object_id(previous.as_bytes()),
                            ObjectKind::TimelineCatalogEvent,
                        )?;
                    }
                    for commit in event.referenced_commits() {
                        self.require_kind(object_id(commit.as_bytes()), ObjectKind::Commit)?;
                    }
                    let operation = CatalogOperationRecord {
                        previous: event.previous,
                        event: event_id,
                    };
                    match operations.insert((event.execution, event.operation), operation) {
                        Some(existing) if existing != operation => {
                            return Err(StoreError::InvalidGraph("TimelineOperationId conflict"));
                        }
                        _ => {}
                    }
                }
                ObjectKind::CheckpointBundleManifest => {
                    if object.schema() != CHECKPOINT_MANIFEST_SCHEMA_V1 {
                        return Err(StoreError::ObjectKind(id));
                    }
                    CheckpointBundleManifestV1::decode(object.payload())
                        .map_err(|error| StoreError::Corrupt(id, error.to_string()))?;
                }
                ObjectKind::TimelineArchiveManifest => {
                    if object.schema() != TIMELINE_ARCHIVE_MANIFEST_SCHEMA_V1 {
                        return Err(StoreError::ObjectKind(id));
                    }
                    let manifest = TimelineArchiveManifestV1::decode(object.payload())
                        .map_err(|error| StoreError::Corrupt(id, error.to_string()))?;
                    self.validate_manifest(&manifest)?;
                }
            }
        }
        self.catalog_operations = operations;
        Ok(())
    }

    fn validate_commit(&self, object: &CheckedObject) -> Result<(), StoreError> {
        let id = object.id();
        if object.schema() != COMMIT_SCHEMA_V1 {
            return Err(StoreError::ObjectKind(id));
        }
        let commit = CommitV1::decode(object.payload())
            .map_err(|error| StoreError::Corrupt(id, error.to_string()))?;
        let (_, program) = self.find_program(commit.program)?;
        let snapshot_object =
            self.require_kind(object_id(commit.snapshot.as_bytes()), ObjectKind::Snapshot)?;
        let state = restore_snapshot(
            snapshot_object.bytes(),
            &program,
            &SnapshotLoadLimits::default(),
        )
        .map_err(|error| StoreError::Corrupt(snapshot_object.id(), error.to_string()))?;
        if state.execution_id != commit.execution || state.turn != commit.turn {
            return Err(StoreError::InvalidGraph("Commit/Snapshot mismatch"));
        }
        match (commit.parent, commit.cause) {
            (None, CommitCauseV1::Genesis) if commit.turn.0 == 0 => {}
            (Some(parent_id), CommitCauseV1::RuntimeTransition(receipt_id)) => {
                let parent_object =
                    self.require_kind(object_id(parent_id.as_bytes()), ObjectKind::Commit)?;
                let parent = CommitV1::decode(parent_object.payload())
                    .map_err(|error| StoreError::Corrupt(parent_object.id(), error.to_string()))?;
                if parent.execution != commit.execution
                    || parent.program != commit.program
                    || parent.turn.0.checked_add(1) != Some(commit.turn.0)
                {
                    return Err(StoreError::InvalidGraph("Commit parent mismatch"));
                }
                let receipt_object =
                    self.require_kind(object_id(receipt_id.as_bytes()), ObjectKind::Receipt)?;
                let receipt = TransitionReceiptV1::decode(receipt_object.payload())
                    .map_err(|error| StoreError::Corrupt(receipt_object.id(), error.to_string()))?;
                if receipt.execution != commit.execution
                    || receipt.program != commit.program
                    || receipt.parent != parent_id
                    || receipt.next_snapshot != commit.snapshot
                    || receipt.next_state != state_digest(&state)
                {
                    return Err(StoreError::InvalidGraph("Commit/Receipt mismatch"));
                }
                let parent_snapshot =
                    self.require_kind(object_id(parent.snapshot.as_bytes()), ObjectKind::Snapshot)?;
                let parent_state = restore_snapshot(
                    parent_snapshot.bytes(),
                    &program,
                    &SnapshotLoadLimits::default(),
                )
                .map_err(|error| StoreError::Corrupt(parent_snapshot.id(), error.to_string()))?;
                if receipt.parent_state != state_digest(&parent_state) {
                    return Err(StoreError::InvalidGraph("Receipt parent state mismatch"));
                }
            }
            _ => return Err(StoreError::InvalidGraph("Commit cause/parent shape")),
        }
        Ok(())
    }

    fn validate_manifest(&self, manifest: &TimelineArchiveManifestV1) -> Result<(), StoreError> {
        self.require_kind(
            object_id(manifest.catalog_head.as_bytes()),
            ObjectKind::TimelineCatalogEvent,
        )?;
        for descriptor in &manifest.objects {
            let object = self.require_object(descriptor.id)?;
            if object.kind() != descriptor.kind
                || object.schema() != descriptor.schema
                || object.bytes().len() as u64 != descriptor.bytes
            {
                return Err(StoreError::InvalidGraph("manifest descriptor mismatch"));
            }
        }
        Ok(())
    }

    fn roots(&self, now: u64) -> Vec<ObjectId> {
        let mut roots = self
            .refs
            .values()
            .map(|value| object_id(value.commit.as_bytes()))
            .chain(
                self.catalogs
                    .values()
                    .map(|value| object_id(value.event.as_bytes())),
            )
            .chain(
                self.archives
                    .values()
                    .map(|value| object_id(value.manifest.as_bytes())),
            )
            .chain(self.pins.values().filter_map(|pin| {
                pin.expires_at
                    .is_none_or(|expires_at| expires_at > now)
                    .then_some(pin.object)
            }))
            .collect::<Vec<_>>();
        roots.sort_unstable();
        roots.dedup();
        roots
    }

    fn edges(&self, object: &CheckedObject) -> Result<Vec<ObjectId>, StoreError> {
        let mut edges = Vec::new();
        match object.kind() {
            ObjectKind::Commit => {
                let value = CommitV1::decode(object.payload())
                    .map_err(|error| StoreError::Corrupt(object.id(), error.to_string()))?;
                if let Some(parent) = value.parent {
                    edges.push(object_id(parent.as_bytes()));
                }
                edges.push(object_id(value.snapshot.as_bytes()));
                if let CommitCauseV1::RuntimeTransition(receipt) = value.cause {
                    edges.push(object_id(receipt.as_bytes()));
                }
                edges.push(self.find_program(value.program)?.0);
            }
            ObjectKind::TimelineCatalogEvent => {
                let value = TimelineCatalogEventV1::decode(object.payload())
                    .map_err(|error| StoreError::Corrupt(object.id(), error.to_string()))?;
                if let Some(previous) = value.previous {
                    edges.push(object_id(previous.as_bytes()));
                }
                edges.extend(
                    value
                        .referenced_commits()
                        .iter()
                        .map(|id| object_id(id.as_bytes())),
                );
            }
            ObjectKind::CheckpointBundleManifest => {
                let value = CheckpointBundleManifestV1::decode(object.payload())
                    .map_err(|error| StoreError::Corrupt(object.id(), error.to_string()))?;
                edges.push(object_id(value.root.as_bytes()));
                edges.extend(value.objects.iter().map(|descriptor| descriptor.id));
                edges.extend(value.optional_host_manifest);
            }
            ObjectKind::TimelineArchiveManifest => {
                let value = TimelineArchiveManifestV1::decode(object.payload())
                    .map_err(|error| StoreError::Corrupt(object.id(), error.to_string()))?;
                edges.push(object_id(value.catalog_head.as_bytes()));
                edges.push(object_id(value.coverage.baseline().as_bytes()));
                edges.extend(value.objects.iter().map(|descriptor| descriptor.id));
                edges.extend(value.host_timeline);
            }
            ObjectKind::Program
            | ObjectKind::Snapshot
            | ObjectKind::Receipt
            | ObjectKind::Value => {}
        }
        edges.sort_unstable();
        edges.dedup();
        Ok(edges)
    }

    fn reachable(&self, roots: &[ObjectId]) -> Result<BTreeSet<ObjectId>, StoreError> {
        let mut marked = BTreeSet::new();
        let mut queue = VecDeque::from(roots.to_vec());
        while let Some(id) = queue.pop_front() {
            if !marked.insert(id) {
                continue;
            }
            let object = self.require_object(id)?;
            queue.extend(self.edges(object)?);
        }
        Ok(marked)
    }
}

impl SaveStore for MemoryStore {
    fn get_object(&self, id: ObjectId) -> Result<Option<CheckedObject>, StoreError> {
        Ok(self.objects.get(&id).map(|stored| stored.object.clone()))
    }

    fn list_objects(&self) -> Result<Vec<CheckedObject>, StoreError> {
        Ok(self
            .objects
            .values()
            .map(|stored| stored.object.clone())
            .collect())
    }

    fn read_ref(&self, key: &RefKey) -> Result<Option<RefValue>, StoreError> {
        Ok(self.refs.get(key).copied())
    }

    fn list_refs(&self) -> Result<Vec<(RefKey, RefValue)>, StoreError> {
        Ok(self
            .refs
            .iter()
            .map(|(key, value)| (key.clone(), *value))
            .collect())
    }

    fn read_catalog_head(
        &self,
        key: &CatalogRefKey,
    ) -> Result<Option<CatalogHeadRefValue>, StoreError> {
        Ok(self.catalogs.get(key).copied())
    }

    fn list_catalog_heads(&self) -> Result<Vec<(CatalogRefKey, CatalogHeadRefValue)>, StoreError> {
        Ok(self
            .catalogs
            .iter()
            .map(|(key, value)| (key.clone(), *value))
            .collect())
    }

    fn read_timeline_archive(
        &self,
        key: &TimelineArchiveRefKey,
    ) -> Result<Option<TimelineArchiveRefValue>, StoreError> {
        Ok(self.archives.get(key).copied())
    }

    fn list_timeline_archives(
        &self,
    ) -> Result<Vec<(TimelineArchiveRefKey, TimelineArchiveRefValue)>, StoreError> {
        Ok(self
            .archives
            .iter()
            .map(|(key, value)| (key.clone(), *value))
            .collect())
    }

    fn read_input(
        &self,
        execution: ExecutionId,
        input: InputId,
    ) -> Result<Option<InputRecord>, StoreError> {
        Ok(self.inputs.get(&(execution, input)).copied())
    }

    fn list_pins(&self) -> Result<Vec<Pin>, StoreError> {
        Ok(self.pins.values().cloned().collect())
    }

    fn commit(&mut self, transaction: CommitTransaction) -> Result<CommitOutcome, StoreError> {
        let mut staged = self.clone();
        let outcome = staged.apply(transaction)?;
        *self = staged;
        Ok(outcome)
    }

    fn collect(&mut self, policy: RetentionPolicy) -> Result<GcReport, StoreError> {
        self.fail(FaultPoint::GcMark)?;
        let roots = self.roots(policy.now);
        let reachable = self.reachable(&roots)?;
        let mut report = GcReport {
            roots: roots.len() as u64,
            reachable: reachable.len() as u64,
            dry_run: policy.dry_run,
            ..GcReport::default()
        };
        let candidates = self
            .objects
            .iter()
            .filter_map(|(id, stored)| {
                let old_enough = stored
                    .inserted_at
                    .checked_add(policy.grace_seconds)
                    .is_some_and(|deadline| deadline <= policy.now);
                (!reachable.contains(id) && old_enough).then_some((*id, stored.object.clone()))
            })
            .collect::<Vec<_>>();
        for (_, object) in &candidates {
            let entry = report.removed.entry(object.kind().code()).or_default();
            entry.objects = entry.objects.saturating_add(1);
            entry.bytes = entry.bytes.saturating_add(object.bytes().len() as u64);
        }
        if !policy.dry_run {
            self.fail(FaultPoint::GcSweep)?;
            for (id, _) in candidates {
                self.objects.remove(&id);
            }
        }
        Ok(report)
    }

    fn integrity_scan(&self) -> Result<Vec<IntegrityIssue>, StoreError> {
        let mut issues = Vec::new();
        for (id, stored) in &self.objects {
            let checked = CheckedObject::from_bytes(
                stored.object.bytes(),
                stored.object.kind(),
                stored.object.schema(),
                stored.object.bytes().len() as u64,
            );
            match checked {
                Ok(object) if object.id() == *id => {}
                Ok(_) => issues.push(IntegrityIssue {
                    object: *id,
                    diagnostic: "ObjectId mismatch".to_owned(),
                }),
                Err(error) => issues.push(IntegrityIssue {
                    object: *id,
                    diagnostic: error.to_string(),
                }),
            }
        }
        Ok(issues)
    }
}

fn object_id(bytes: &[u8; 32]) -> ObjectId {
    ObjectId::from_bytes(*bytes)
}

#[allow(dead_code)]
fn _manifest_error_is_send_sync(_: &ManifestError) {}
