use std::collections::{BTreeMap, BTreeSet, VecDeque};

use narrata_core::{
    EffectId, ExecutionId, InputId, ObjectId, ProgramArtifactId, RewindPolicy,
    TimelineCatalogEventId,
    codec::ObjectKind,
    limits::{ProgramLoadLimits, SnapshotLoadLimits},
    program::load_program,
    snapshot::{restore_snapshot, state_digest},
};

use crate::{
    CATALOG_EVENT_SCHEMA_V1, CHECKPOINT_MANIFEST_SCHEMA_V1, COMPOUND_SAVE_MANIFEST_SCHEMA_V1,
    CatalogHeadRefValue, CatalogRefKey, CheckedObject, CommitCauseV1, CommitOutcome,
    CommitTransaction, CommitV1, CompoundSaveManifestV1, CompoundSaveRefKey, CompoundSaveRefValue,
    EFFECT_RESPONSE_SCHEMA_V1, EffectClaim, EffectClaimResult, EffectLedgerEntry, EffectOutcome,
    EffectOutcomeRecord, EffectStoreError, FaultPoint, GcReport, HOST_TIMELINE_MANIFEST_SCHEMA_V1,
    HostTimelineManifestV1, InputIdConflict, InputRecord, IntegrityIssue, LeaseId, LedgerFence,
    LedgerStatus, ManifestError, Pin, PutOutcome, RecordedEffectResponseV1, RefKey, RefValue,
    RetentionPolicy, STORED_RECEIPT_SCHEMA_V1, SaveStore, StoreError,
    TIMELINE_ARCHIVE_MANIFEST_SCHEMA_V1, TimelineArchiveManifestV1, TimelineArchiveRefKey,
    TimelineArchiveRefValue, TimelineCatalogEventV1, TransitionReceiptV1,
    commit::COMMIT_SCHEMA_V1,
    manifest::CheckpointBundleManifestV1,
    store::{
        CatalogOperationIndex, CatalogOperationRecord, check_expected_archive,
        check_expected_catalog, check_expected_compound_save, check_expected_ref, next_revision,
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
    pub effects: Vec<EffectLedgerEntry>,
    pub ledger_fences: Vec<(ExecutionId, LedgerFence)>,
    pub compound_saves: Vec<(CompoundSaveRefKey, CompoundSaveRefValue)>,
}

#[derive(Clone, Debug, Default)]
pub struct MemoryStore {
    objects: BTreeMap<ObjectId, StoredObject>,
    refs: BTreeMap<RefKey, RefValue>,
    catalogs: BTreeMap<CatalogRefKey, CatalogHeadRefValue>,
    archives: BTreeMap<TimelineArchiveRefKey, TimelineArchiveRefValue>,
    inputs: BTreeMap<(ExecutionId, InputId), InputRecord>,
    pins: BTreeMap<(String, ObjectId), Pin>,
    effects: BTreeMap<(ExecutionId, EffectId), EffectLedgerEntry>,
    ledger_fences: BTreeMap<ExecutionId, LedgerFence>,
    compound_saves: BTreeMap<CompoundSaveRefKey, CompoundSaveRefValue>,
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
        store.effects = state
            .effects
            .into_iter()
            .map(|entry| ((entry.execution, entry.effect), entry))
            .collect();
        store.ledger_fences = state.ledger_fences.into_iter().collect();
        store.compound_saves = state.compound_saves.into_iter().collect();
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
        for entry in store.effects.values() {
            store.require_kind(
                object_id(entry.origin_commit.as_bytes()),
                ObjectKind::Commit,
            )?;
            if let Some(response) = entry.status.response() {
                store.require_kind(response, ObjectKind::EffectResponse)?;
            }
        }
        for value in store.compound_saves.values() {
            store.require_kind(
                object_id(value.manifest.as_bytes()),
                ObjectKind::CompoundSaveManifest,
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
            effects: self.effects.values().cloned().collect(),
            ledger_fences: self
                .ledger_fences
                .iter()
                .map(|(execution, fence)| (*execution, *fence))
                .collect(),
            compound_saves: self
                .compound_saves
                .iter()
                .map(|(key, value)| (key.clone(), *value))
                .collect(),
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
                | ObjectKind::TimelineArchiveManifest
                | ObjectKind::EffectResponse
                | ObjectKind::CompoundSaveManifest
                | ObjectKind::HostTimelineManifest => FaultPoint::EdgeIndexWrite,
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

        for mutation in transaction.compound_saves {
            let actual = self.compound_saves.get(&mutation.key).copied();
            check_expected_compound_save(&mutation.key, mutation.expected, actual, mutation.next)?;
            let next = match mutation.next {
                Some(manifest) => {
                    self.require_kind(
                        object_id(manifest.as_bytes()),
                        ObjectKind::CompoundSaveManifest,
                    )?;
                    let value = CompoundSaveRefValue {
                        revision: next_revision(actual.map(|value| value.revision))?,
                        manifest,
                    };
                    self.compound_saves.insert(mutation.key.clone(), value);
                    Some(value)
                }
                None => {
                    self.compound_saves.remove(&mutation.key);
                    None
                }
            };
            outcome.compound_saves.insert(mutation.key, next);
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

    fn apply_claim(&mut self, claim: EffectClaim) -> Result<EffectClaimResult, StoreError> {
        self.fail(FaultPoint::LedgerClaim)?;
        if claim.expires_at <= claim.now {
            return Err(EffectStoreError::InvalidLease.into());
        }
        self.require_kind(
            object_id(claim.origin_commit.as_bytes()),
            ObjectKind::Commit,
        )?;
        let key = (claim.execution, claim.effect);
        if let Some(existing) = self.effects.get(&key).cloned() {
            if !same_claim_contract(&existing, &claim) {
                return Err(EffectStoreError::RequestConflict.into());
            }
            match existing.status {
                LedgerStatus::Claimed { lease, expires_at }
                    if lease != claim.lease && expires_at > claim.now =>
                {
                    return Ok(EffectClaimResult::Leased { lease, expires_at });
                }
                LedgerStatus::Claimed { .. } | LedgerStatus::RetryableFailure { .. } => {}
                LedgerStatus::Completed { .. }
                | LedgerStatus::Rejected { .. }
                | LedgerStatus::UnknownOutcome { .. }
                | LedgerStatus::Compensated { .. } => {
                    return Ok(EffectClaimResult::Recorded(existing));
                }
            }
        }
        let entry = EffectLedgerEntry {
            execution: claim.execution,
            effect: claim.effect,
            request_digest: claim.request_digest,
            capability: claim.capability,
            capability_version: claim.capability_version,
            origin_commit: claim.origin_commit,
            delivery: claim.delivery,
            rewind: claim.rewind,
            status: LedgerStatus::Claimed {
                lease: claim.lease,
                expires_at: claim.expires_at,
            },
        };
        self.effects.insert(key, entry.clone());
        Ok(EffectClaimResult::Claimed(entry))
    }

    fn apply_renew(
        &mut self,
        execution: ExecutionId,
        effect: EffectId,
        lease: LeaseId,
        now: u64,
        expires_at: u64,
    ) -> Result<EffectLedgerEntry, StoreError> {
        self.fail(FaultPoint::LedgerClaim)?;
        if expires_at <= now {
            return Err(EffectStoreError::InvalidLease.into());
        }
        let entry = self
            .effects
            .get_mut(&(execution, effect))
            .ok_or(EffectStoreError::Missing)?;
        match entry.status {
            LedgerStatus::Claimed {
                lease: actual,
                expires_at: actual_expiry,
            } if actual == lease && actual_expiry > now => {
                entry.status = LedgerStatus::Claimed { lease, expires_at };
                Ok(entry.clone())
            }
            _ => Err(EffectStoreError::LeaseMismatch.into()),
        }
    }

    fn apply_outcome(
        &mut self,
        record: EffectOutcomeRecord,
    ) -> Result<EffectLedgerEntry, StoreError> {
        self.fail(FaultPoint::LedgerOutcome)?;
        let key = (record.execution, record.effect);
        let existing = self
            .effects
            .get(&key)
            .cloned()
            .ok_or(EffectStoreError::Missing)?;
        if existing.request_digest != record.request_digest {
            return Err(EffectStoreError::RequestConflict.into());
        }
        match existing.status {
            LedgerStatus::Claimed { lease, expires_at }
                if lease == record.lease && expires_at > record.observed_at => {}
            LedgerStatus::Claimed { .. } | LedgerStatus::RetryableFailure { .. } => {
                return Err(EffectStoreError::LeaseMismatch.into());
            }
            LedgerStatus::Completed { .. }
            | LedgerStatus::Rejected { .. }
            | LedgerStatus::UnknownOutcome { .. }
            | LedgerStatus::Compensated { .. } => {
                return Err(EffectStoreError::Terminal.into());
            }
        }
        let status = match record.outcome {
            EffectOutcome::Completed(response) => {
                let response = self.record_response(&existing, response, record.observed_at)?;
                LedgerStatus::Completed {
                    response,
                    fence: self.allocate_fence(record.execution)?,
                }
            }
            EffectOutcome::Rejected(response) => {
                let response = self.record_response(&existing, response, record.observed_at)?;
                LedgerStatus::Rejected {
                    response,
                    fence: self.allocate_fence(record.execution)?,
                }
            }
            EffectOutcome::RetryableFailure(diagnostic) => {
                LedgerStatus::RetryableFailure { diagnostic }
            }
            EffectOutcome::UnknownOutcome(diagnostic) => LedgerStatus::UnknownOutcome {
                diagnostic,
                fence: self.allocate_fence(record.execution)?,
            },
        };
        let entry = self
            .effects
            .get_mut(&key)
            .ok_or(EffectStoreError::Missing)?;
        entry.status = status;
        Ok(entry.clone())
    }

    fn record_response(
        &mut self,
        entry: &EffectLedgerEntry,
        response: RecordedEffectResponseV1,
        observed_at: u64,
    ) -> Result<ObjectId, StoreError> {
        if response.effect != entry.effect
            || response.request_digest != entry.request_digest
            || response.capability != entry.capability
            || response.capability_version != entry.capability_version
        {
            return Err(EffectStoreError::InvalidResponse("contract mismatch".to_owned()).into());
        }
        let object = response.to_object();
        let id = object.id();
        self.put_object(object, observed_at)?;
        Ok(id)
    }

    fn allocate_fence(&mut self, execution: ExecutionId) -> Result<LedgerFence, StoreError> {
        let next = self
            .ledger_fences
            .get(&execution)
            .copied()
            .unwrap_or_else(LedgerFence::zero)
            .next()?;
        self.ledger_fences.insert(execution, next);
        Ok(next)
    }

    fn apply_compensation(
        &mut self,
        execution: ExecutionId,
        original: EffectId,
        by_effect: EffectId,
    ) -> Result<EffectLedgerEntry, StoreError> {
        self.fail(FaultPoint::LedgerOutcome)?;
        let compensator = self
            .effects
            .get(&(execution, by_effect))
            .cloned()
            .ok_or(EffectStoreError::Missing)?;
        let compensation_fence = match compensator.status {
            LedgerStatus::Completed { fence, .. } => fence,
            _ => return Err(EffectStoreError::InvalidCompensation.into()),
        };
        let entry = self
            .effects
            .get_mut(&(execution, original))
            .ok_or(EffectStoreError::Missing)?;
        let RewindPolicy::Compensatable { capability } = &entry.rewind else {
            return Err(EffectStoreError::InvalidCompensation.into());
        };
        if capability != &compensator.capability {
            return Err(EffectStoreError::InvalidCompensation.into());
        }
        let (response, original_fence) = match entry.status {
            LedgerStatus::Completed { response, fence } => (response, fence),
            _ => return Err(EffectStoreError::InvalidCompensation.into()),
        };
        entry.status = LedgerStatus::Compensated {
            response,
            original_fence,
            by_effect,
            compensation_fence,
        };
        Ok(entry.clone())
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
                ObjectKind::EffectResponse => {
                    if object.schema() != EFFECT_RESPONSE_SCHEMA_V1 {
                        return Err(StoreError::ObjectKind(id));
                    }
                    RecordedEffectResponseV1::decode(object.payload())
                        .map_err(|error| StoreError::Corrupt(id, error.to_string()))?;
                }
                ObjectKind::CompoundSaveManifest => {
                    if object.schema() != COMPOUND_SAVE_MANIFEST_SCHEMA_V1 {
                        return Err(StoreError::ObjectKind(id));
                    }
                    let manifest = CompoundSaveManifestV1::decode(object.payload())
                        .map_err(|error| StoreError::Corrupt(id, error.to_string()))?;
                    let commit_object = self.require_kind(
                        object_id(manifest.narrative.as_bytes()),
                        ObjectKind::Commit,
                    )?;
                    let commit = CommitV1::decode(commit_object.payload()).map_err(|error| {
                        StoreError::Corrupt(commit_object.id(), error.to_string())
                    })?;
                    if commit.execution != manifest.execution
                        || commit.program != manifest.program
                        || commit.ledger_fence != manifest.ledger_fence.get()
                    {
                        return Err(StoreError::InvalidGraph("Compound Save mismatch"));
                    }
                }
                ObjectKind::HostTimelineManifest => {
                    if object.schema() != HOST_TIMELINE_MANIFEST_SCHEMA_V1 {
                        return Err(StoreError::ObjectKind(id));
                    }
                    let manifest = HostTimelineManifestV1::decode(object.payload())
                        .map_err(|error| StoreError::Corrupt(id, error.to_string()))?;
                    for entry in &manifest.entries {
                        let commit_object = self.require_kind(
                            object_id(entry.narrative.as_bytes()),
                            ObjectKind::Commit,
                        )?;
                        let commit =
                            CommitV1::decode(commit_object.payload()).map_err(|error| {
                                StoreError::Corrupt(commit_object.id(), error.to_string())
                            })?;
                        if commit.execution != manifest.execution
                            || commit.ledger_fence != entry.ledger_fence.get()
                        {
                            return Err(StoreError::InvalidGraph("Host Timeline mismatch"));
                        }
                    }
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
            (None, CommitCauseV1::Genesis) if commit.turn.0 == 0 && commit.ledger_fence == 0 => {}
            (Some(parent_id), CommitCauseV1::RuntimeTransition(receipt_id)) => {
                let parent_object =
                    self.require_kind(object_id(parent_id.as_bytes()), ObjectKind::Commit)?;
                let parent = CommitV1::decode(parent_object.payload())
                    .map_err(|error| StoreError::Corrupt(parent_object.id(), error.to_string()))?;
                if parent.execution != commit.execution
                    || parent.program != commit.program
                    || parent.turn.0.checked_add(1) != Some(commit.turn.0)
                    || commit.ledger_fence < parent.ledger_fence
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
                let parent_object =
                    self.require_kind(object_id(parent_id.as_bytes()), ObjectKind::Commit)?;
                let parent = CommitV1::decode(parent_object.payload())
                    .map_err(|error| StoreError::Corrupt(parent_object.id(), error.to_string()))?;
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
            .chain(self.effects.values().flat_map(|entry| {
                std::iter::once(object_id(entry.origin_commit.as_bytes()))
                    .chain(entry.status.response())
            }))
            .chain(
                self.compound_saves
                    .values()
                    .map(|value| object_id(value.manifest.as_bytes())),
            )
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
            ObjectKind::CompoundSaveManifest => {
                let value = CompoundSaveManifestV1::decode(object.payload())
                    .map_err(|error| StoreError::Corrupt(object.id(), error.to_string()))?;
                edges.push(object_id(value.narrative.as_bytes()));
            }
            ObjectKind::HostTimelineManifest => {
                let value = HostTimelineManifestV1::decode(object.payload())
                    .map_err(|error| StoreError::Corrupt(object.id(), error.to_string()))?;
                edges.extend(
                    value
                        .entries
                        .iter()
                        .map(|entry| object_id(entry.narrative.as_bytes())),
                );
            }
            ObjectKind::Program
            | ObjectKind::Snapshot
            | ObjectKind::Receipt
            | ObjectKind::Value
            | ObjectKind::EffectResponse => {}
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

    fn read_compound_save(
        &self,
        key: &CompoundSaveRefKey,
    ) -> Result<Option<CompoundSaveRefValue>, StoreError> {
        Ok(self.compound_saves.get(key).copied())
    }

    fn list_compound_saves(
        &self,
    ) -> Result<Vec<(CompoundSaveRefKey, CompoundSaveRefValue)>, StoreError> {
        Ok(self
            .compound_saves
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

    fn read_effect(
        &self,
        execution: ExecutionId,
        effect: EffectId,
    ) -> Result<Option<EffectLedgerEntry>, StoreError> {
        Ok(self.effects.get(&(execution, effect)).cloned())
    }

    fn list_effects(&self, execution: ExecutionId) -> Result<Vec<EffectLedgerEntry>, StoreError> {
        Ok(self
            .effects
            .range((execution, EffectId::from_bytes([0; 32]))..)
            .take_while(|((candidate, _), _)| candidate == &execution)
            .map(|(_, entry)| entry.clone())
            .collect())
    }

    fn current_ledger_fence(&self, execution: ExecutionId) -> Result<LedgerFence, StoreError> {
        Ok(self
            .ledger_fences
            .get(&execution)
            .copied()
            .unwrap_or_else(LedgerFence::zero))
    }

    fn claim_effect(&mut self, claim: EffectClaim) -> Result<EffectClaimResult, StoreError> {
        let mut staged = self.clone();
        let result = staged.apply_claim(claim)?;
        *self = staged;
        Ok(result)
    }

    fn renew_effect_lease(
        &mut self,
        execution: ExecutionId,
        effect: EffectId,
        lease: LeaseId,
        now: u64,
        expires_at: u64,
    ) -> Result<EffectLedgerEntry, StoreError> {
        let mut staged = self.clone();
        let result = staged.apply_renew(execution, effect, lease, now, expires_at)?;
        *self = staged;
        Ok(result)
    }

    fn record_effect_outcome(
        &mut self,
        record: EffectOutcomeRecord,
    ) -> Result<EffectLedgerEntry, StoreError> {
        let mut staged = self.clone();
        let result = staged.apply_outcome(record)?;
        *self = staged;
        Ok(result)
    }

    fn mark_effect_compensated(
        &mut self,
        execution: ExecutionId,
        original: EffectId,
        by_effect: EffectId,
    ) -> Result<EffectLedgerEntry, StoreError> {
        let mut staged = self.clone();
        let result = staged.apply_compensation(execution, original, by_effect)?;
        *self = staged;
        Ok(result)
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

fn same_claim_contract(entry: &EffectLedgerEntry, claim: &EffectClaim) -> bool {
    entry.request_digest == claim.request_digest
        && entry.capability == claim.capability
        && entry.capability_version == claim.capability_version
        && entry.origin_commit == claim.origin_commit
        && entry.delivery == claim.delivery
        && entry.rewind == claim.rewind
}

#[allow(dead_code)]
fn _manifest_error_is_send_sync(_: &ManifestError) {}
