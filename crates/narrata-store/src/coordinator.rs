use std::{collections::BTreeSet, sync::Arc};

use narrata_core::{
    CheckedProgram, CommitId, ExecutionId, ObjectId, ProgramArtifactId, ReceiptId, SnapshotId,
    TimelineArchiveManifestId, TimelineCatalogEventId,
    codec::ObjectKind,
    limits::{MacrostepLimits, ProgramLoadLimits, SnapshotLoadLimits},
    program::{encode_program_artifact, load_program},
    runtime::{
        CheckedRuntimeInput, DraftResult, RuntimeFault, RuntimeStateV0, SliceBudget, SliceOutcome,
        TransitionStartError, begin_transition, new_execution,
    },
    snapshot::{export_snapshot, restore_snapshot},
};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    ArchiveMutation, ArchiveRefName, ArchivedBookmark, ArchivedBranchRef, ArchivedRefSnapshot,
    ArchivedSaveRef, BranchId, CATALOG_EVENT_SCHEMA_V1, CatalogHeadRefValue, CatalogMutation,
    CatalogRefKey, CheckedObject, CommitCauseV1, CommitTransaction, CommitV1, InputIdConflict,
    InputRecord, NameError, RefKey, RefMutation, RefName, RefRevision, RefValue, SaveStore,
    StoreError, TimelineArchiveBundle, TimelineArchiveRefKey, TimelineCatalogEventKind,
    TimelineCatalogEventV1, TimelineCoverage, TimelineOperationId, TimelineRecordingMode,
    TransitionReceiptV1,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InitialRecordingMode {
    Standard,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimelineSession {
    pub selected_branch: BranchId,
    pub cursor: CommitId,
}

#[derive(Clone, Debug)]
pub struct LoadedCommit {
    pub id: CommitId,
    pub commit: CommitV1,
    pub state: RuntimeStateV0,
}

#[derive(Clone, Debug)]
pub struct CommittedRunResult {
    pub commit: CommitId,
    pub receipt: ReceiptId,
    pub branch: BranchId,
    pub reused: bool,
    pub state: Arc<RuntimeStateV0>,
    pub result: DraftResult,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SaveResult {
    Saved(RefValue),
    Deferred(SaveDeferred),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SaveDeferred {
    pub slot: RefName,
    pub expected: Option<RefRevision>,
}

#[derive(Clone, Debug, Error)]
pub enum CoordinatorError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("Runtime initialization failed: {0}")]
    Init(String),
    #[error(transparent)]
    Start(#[from] TransitionStartError),
    #[error("Runtime transition faulted: {0}")]
    Runtime(#[from] RuntimeFault),
    #[error("Snapshot operation failed: {0}")]
    Snapshot(String),
    #[error("stored object is invalid: {0}")]
    Object(String),
    #[error("stored Commit is corrupt: {0}")]
    Commit(String),
    #[error("Catalog operation is invalid: {0}")]
    Catalog(String),
    #[error(transparent)]
    Name(#[from] NameError),
    #[error("Commit does not belong to this Execution or Program")]
    IncompatibleCommit,
    #[error("target Commit is not an ancestor of the selected branch")]
    NotOnSelectedBranch,
    #[error("redo is ambiguous; choose one of {0:?}")]
    RedoAmbiguous(Vec<CommitId>),
    #[error("there is no later Commit on the selected branch")]
    NoRedo,
    #[error("complete timeline recording is already enabled")]
    AlreadyComplete,
    #[error("complete timeline recording is not enabled")]
    NotComplete,
    #[error("requested Ref is missing")]
    MissingRef,
}

pub struct SessionCoordinator<S> {
    store: S,
    program: Arc<CheckedProgram>,
    execution: ExecutionId,
    session_key: RefKey,
    branch_key: RefKey,
    branch_revision: RefRevision,
    active_revision: RefRevision,
    catalog_revision: Option<RefRevision>,
    recording: TimelineRecordingMode,
    timeline: TimelineSession,
    state: Arc<RuntimeStateV0>,
}

impl<S: SaveStore> SessionCoordinator<S> {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        mut store: S,
        program: Arc<CheckedProgram>,
        execution: ExecutionId,
        session_name: RefName,
        branch: BranchId,
        recording: InitialRecordingMode,
        observed_at: u64,
    ) -> Result<Self, CoordinatorError> {
        let state = Arc::new(
            new_execution(&program, execution)
                .map_err(|error| CoordinatorError::Init(error.to_string()))?,
        );
        let program_object = checked_existing_object(
            &encode_program_artifact(program.artifact()),
            ObjectKind::Program,
            0,
        )?;
        let snapshot_object = checked_existing_object(
            &export_snapshot(&state)
                .map_err(|error| CoordinatorError::Snapshot(error.to_string()))?,
            ObjectKind::Snapshot,
            0,
        )?;
        let snapshot = SnapshotId::from_bytes(*snapshot_object.id().as_bytes());
        let commit = CommitV1 {
            parent: None,
            execution,
            program: program.artifact_id(),
            snapshot,
            cause: CommitCauseV1::Genesis,
            ledger_fence: 0,
            turn: state.turn,
        };
        let commit_object = commit.to_object();
        let commit_id = CommitId::from_bytes(*commit_object.id().as_bytes());
        let branch_key = RefKey::branch(execution, branch)?;
        let session_key = RefKey::active(session_name)?;
        let mut transaction = CommitTransaction {
            objects: vec![program_object, snapshot_object, commit_object],
            refs: vec![
                RefMutation {
                    key: branch_key.clone(),
                    expected: None,
                    next: Some(commit_id),
                },
                RefMutation {
                    key: session_key.clone(),
                    expected: None,
                    next: Some(commit_id),
                },
            ],
            observed_at,
            ..CommitTransaction::default()
        };
        let (recording, catalog_revision) = match recording {
            InitialRecordingMode::Standard => (TimelineRecordingMode::Standard, None),
            InitialRecordingMode::Complete => {
                let coverage = TimelineCoverage::FromBaseline {
                    baseline: commit_id,
                };
                let event = TimelineCatalogEventV1 {
                    execution,
                    previous: None,
                    operation: deterministic_operation(b"recording-started", commit_id.as_bytes()),
                    kind: TimelineCatalogEventKind::RecordingStarted {
                        baseline: commit_id,
                        initial_refs: vec![ArchivedRefSnapshot::Branch(ArchivedBranchRef {
                            branch,
                            head: commit_id,
                        })],
                    },
                };
                let event_object = event
                    .to_object()
                    .map_err(|error| CoordinatorError::Catalog(error.to_string()))?;
                let event_id = TimelineCatalogEventId::from_bytes(*event_object.id().as_bytes());
                transaction.objects.push(event_object);
                transaction.catalogs.push(CatalogMutation {
                    key: CatalogRefKey::new(execution),
                    expected: None,
                    next: Some(event_id),
                    coverage,
                });
                (
                    TimelineRecordingMode::Complete { coverage },
                    Some(RefRevision::initial()),
                )
            }
        };
        store.commit(transaction)?;
        Ok(Self {
            store,
            program,
            execution,
            session_key,
            branch_key,
            branch_revision: RefRevision::initial(),
            active_revision: RefRevision::initial(),
            catalog_revision,
            recording,
            timeline: TimelineSession {
                selected_branch: branch,
                cursor: commit_id,
            },
            state,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn open(
        store: S,
        program: Arc<CheckedProgram>,
        execution: ExecutionId,
        session_name: RefName,
        selected_branch: BranchId,
    ) -> Result<Self, CoordinatorError> {
        let branch_key = RefKey::branch(execution, selected_branch)?;
        let session_key = RefKey::active(session_name)?;
        let branch_value = store
            .read_ref(&branch_key)?
            .ok_or(CoordinatorError::MissingRef)?;
        let active_value = store
            .read_ref(&session_key)?
            .ok_or(CoordinatorError::MissingRef)?;
        let loaded = load_commit(&store, active_value.commit, &program)?;
        if loaded.commit.execution != execution {
            return Err(CoordinatorError::IncompatibleCommit);
        }
        ensure_ancestor(&store, active_value.commit, branch_value.commit)?;
        let catalog = store.read_catalog_head(&CatalogRefKey::new(execution))?;
        let (recording, catalog_revision) =
            catalog.map_or((TimelineRecordingMode::Standard, None), |value| {
                (
                    TimelineRecordingMode::Complete {
                        coverage: value.coverage,
                    },
                    Some(value.revision),
                )
            });
        Ok(Self {
            store,
            program,
            execution,
            session_key,
            branch_key,
            branch_revision: branch_value.revision,
            active_revision: active_value.revision,
            catalog_revision,
            recording,
            timeline: TimelineSession {
                selected_branch,
                cursor: active_value.commit,
            },
            state: Arc::new(loaded.state),
        })
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }

    pub fn into_store(self) -> S {
        self.store
    }

    pub const fn timeline(&self) -> TimelineSession {
        self.timeline
    }

    pub fn state(&self) -> &Arc<RuntimeStateV0> {
        &self.state
    }

    pub fn recording(&self) -> &TimelineRecordingMode {
        &self.recording
    }

    pub fn dispatch(
        &mut self,
        input: CheckedRuntimeInput,
        limits: MacrostepLimits,
        observed_at: u64,
    ) -> Result<CommittedRunResult, CoordinatorError> {
        let input_id = input.request_id();
        let payload = input.payload_digest();
        if let Some(existing) = self.store.read_input(self.execution, input_id)? {
            let proposed = InputRecord {
                execution: self.execution,
                input: input_id,
                parent: self.timeline.cursor,
                payload,
                commit: existing.commit,
            };
            if existing.parent != proposed.parent || existing.payload != proposed.payload {
                return Err(StoreError::from(InputIdConflict { existing, proposed }).into());
            }
            let loaded = load_commit(&self.store, existing.commit, &self.program)?;
            let receipt_id = match loaded.commit.cause {
                CommitCauseV1::RuntimeTransition(id) => id,
                CommitCauseV1::Genesis => {
                    return Err(CoordinatorError::Commit(
                        "input index points to Genesis".to_owned(),
                    ));
                }
            };
            let result = result_from_state(&loaded.state)?;
            let active = self
                .store
                .read_ref(&self.session_key)?
                .ok_or(CoordinatorError::MissingRef)?;
            if active.commit != existing.commit {
                let outcome = self.store.commit(CommitTransaction {
                    refs: vec![RefMutation {
                        key: self.session_key.clone(),
                        expected: Some(active.revision),
                        next: Some(existing.commit),
                    }],
                    ..CommitTransaction::default()
                })?;
                self.active_revision = outcome
                    .refs
                    .get(&self.session_key)
                    .and_then(|value| *value)
                    .map(|value| value.revision)
                    .ok_or(CoordinatorError::MissingRef)?;
            } else {
                self.active_revision = active.revision;
            }
            self.state = Arc::new(loaded.state);
            self.timeline.cursor = existing.commit;
            if let Some(branch) = self.store.read_ref(&self.branch_key)?
                && branch.commit == existing.commit
            {
                self.branch_revision = branch.revision;
            }
            return Ok(CommittedRunResult {
                commit: existing.commit,
                receipt: receipt_id,
                branch: self.timeline.selected_branch,
                reused: true,
                state: self.state.clone(),
                result,
            });
        }

        let draft = run_transition(self.program.clone(), self.state.clone(), input, limits)?;
        let result = draft.result().clone();
        let snapshot_bytes = export_snapshot(draft.next_state())
            .map_err(|error| CoordinatorError::Snapshot(error.to_string()))?;
        let snapshot_object = checked_existing_object(&snapshot_bytes, ObjectKind::Snapshot, 0)?;
        let snapshot_id = SnapshotId::from_bytes(*snapshot_object.id().as_bytes());
        let receipt = TransitionReceiptV1::from_draft(
            self.execution,
            self.program.artifact_id(),
            self.timeline.cursor,
            snapshot_id,
            &draft,
        );
        let receipt_object = receipt.to_object();
        let receipt_id = ReceiptId::from_bytes(*receipt_object.id().as_bytes());
        let commit = CommitV1 {
            parent: Some(self.timeline.cursor),
            execution: self.execution,
            program: self.program.artifact_id(),
            snapshot: snapshot_id,
            cause: CommitCauseV1::RuntimeTransition(receipt_id),
            ledger_fence: 0,
            turn: draft.next_state().turn,
        };
        let commit_object = commit.to_object();
        let commit_id = CommitId::from_bytes(*commit_object.id().as_bytes());
        let current_branch = self
            .store
            .read_ref(&self.branch_key)?
            .ok_or(CoordinatorError::MissingRef)?;
        let forking = current_branch.commit != self.timeline.cursor;
        let previous_cursor = self.timeline.cursor;
        let mut next_branch = self.timeline.selected_branch;
        let mut next_branch_key = self.branch_key.clone();
        let mut branch_expected = Some(self.branch_revision);
        if forking {
            ensure_ancestor(&self.store, self.timeline.cursor, current_branch.commit)?;
            next_branch = deterministic_branch(self.timeline.cursor, payload.as_bytes());
            next_branch_key = RefKey::branch(self.execution, next_branch)?;
            branch_expected = None;
        } else if current_branch.revision != self.branch_revision {
            return Err(StoreError::from(crate::RefConflict {
                key: self.branch_key.storage_key(),
                expected: Some(self.branch_revision),
                actual: Some(current_branch),
                proposed: Some(commit_id),
            })
            .into());
        }

        let mut transaction = CommitTransaction {
            objects: vec![snapshot_object, receipt_object, commit_object],
            refs: vec![
                RefMutation {
                    key: next_branch_key.clone(),
                    expected: branch_expected,
                    next: Some(commit_id),
                },
                RefMutation {
                    key: self.session_key.clone(),
                    expected: Some(self.active_revision),
                    next: Some(commit_id),
                },
            ],
            inputs: vec![InputRecord {
                execution: self.execution,
                input: input_id,
                parent: previous_cursor,
                payload,
                commit: commit_id,
            }],
            observed_at,
            ..CommitTransaction::default()
        };
        if let TimelineRecordingMode::Complete { coverage } = self.recording {
            let kind = if forking {
                TimelineCatalogEventKind::BranchCreated {
                    branch: next_branch,
                    parent: previous_cursor,
                    head: commit_id,
                }
            } else {
                TimelineCatalogEventKind::BranchAdvanced {
                    branch: next_branch,
                    previous_head: previous_cursor,
                    next_head: commit_id,
                }
            };
            append_catalog(
                &self.store,
                self.execution,
                self.catalog_revision,
                coverage,
                TimelineOperationId::from_bytes(*input_id.as_bytes()),
                kind,
                &mut transaction,
            )?;
        }
        let outcome = self.store.commit(transaction)?;
        self.branch_revision = outcome
            .refs
            .get(&next_branch_key)
            .and_then(|value| *value)
            .map(|value| value.revision)
            .ok_or(CoordinatorError::MissingRef)?;
        self.active_revision = outcome
            .refs
            .get(&self.session_key)
            .and_then(|value| *value)
            .map(|value| value.revision)
            .ok_or(CoordinatorError::MissingRef)?;
        if let Some(value) = outcome.catalogs.get(&CatalogRefKey::new(self.execution)) {
            self.catalog_revision = value.map(|value| value.revision);
        }
        self.branch_key = next_branch_key;
        self.timeline = TimelineSession {
            selected_branch: next_branch,
            cursor: commit_id,
        };
        self.state = Arc::new(draft.into_next_state());
        Ok(CommittedRunResult {
            commit: commit_id,
            receipt: receipt_id,
            branch: next_branch,
            reused: false,
            state: self.state.clone(),
            result,
        })
    }

    pub fn rewind_to(
        &mut self,
        target: CommitId,
        observed_at: u64,
    ) -> Result<LoadedCommit, CoordinatorError> {
        let head = self
            .store
            .read_ref(&self.branch_key)?
            .ok_or(CoordinatorError::MissingRef)?;
        ensure_ancestor(&self.store, target, head.commit)?;
        let loaded = load_commit(&self.store, target, &self.program)?;
        let outcome = self.store.commit(CommitTransaction {
            refs: vec![RefMutation {
                key: self.session_key.clone(),
                expected: Some(self.active_revision),
                next: Some(target),
            }],
            observed_at,
            ..CommitTransaction::default()
        })?;
        self.active_revision = outcome
            .refs
            .get(&self.session_key)
            .and_then(|value| *value)
            .map(|value| value.revision)
            .ok_or(CoordinatorError::MissingRef)?;
        self.timeline.cursor = target;
        self.state = Arc::new(loaded.state.clone());
        Ok(loaded)
    }

    pub fn rewind(
        &mut self,
        steps: u64,
        observed_at: u64,
    ) -> Result<LoadedCommit, CoordinatorError> {
        let mut target = self.timeline.cursor;
        for _ in 0..steps {
            let commit = read_commit(&self.store, target)?;
            target = commit.parent.ok_or(CoordinatorError::NotOnSelectedBranch)?;
        }
        self.rewind_to(target, observed_at)
    }

    pub fn redo(&mut self, steps: u64, observed_at: u64) -> Result<LoadedCommit, CoordinatorError> {
        let head = self
            .store
            .read_ref(&self.branch_key)?
            .ok_or(CoordinatorError::MissingRef)?;
        let mut path = path_from_ancestor(&self.store, self.timeline.cursor, head.commit)?;
        if path.is_empty() {
            return Err(CoordinatorError::NoRedo);
        }
        let index = usize::try_from(steps.saturating_sub(1))
            .unwrap_or(usize::MAX)
            .min(path.len().saturating_sub(1));
        let target = path.swap_remove(index);
        self.rewind_to(target, observed_at)
    }

    pub fn redo_candidates(&self) -> Result<Vec<CommitId>, CoordinatorError> {
        let mut values = self
            .store
            .list_objects()?
            .into_iter()
            .filter(|object| object.kind() == ObjectKind::Commit)
            .filter_map(|object| {
                CommitV1::decode(object.payload())
                    .ok()
                    .filter(|commit| commit.parent == Some(self.timeline.cursor))
                    .map(|_| CommitId::from_bytes(*object.id().as_bytes()))
            })
            .collect::<Vec<_>>();
        values.sort_unstable();
        Ok(values)
    }

    pub fn fork(
        &mut self,
        branch: BranchId,
        operation: TimelineOperationId,
        observed_at: u64,
    ) -> Result<RefValue, CoordinatorError> {
        let key = RefKey::branch(self.execution, branch)?;
        let mut transaction = CommitTransaction {
            refs: vec![RefMutation {
                key: key.clone(),
                expected: None,
                next: Some(self.timeline.cursor),
            }],
            observed_at,
            ..CommitTransaction::default()
        };
        if let TimelineRecordingMode::Complete { coverage } = self.recording {
            append_catalog(
                &self.store,
                self.execution,
                self.catalog_revision,
                coverage,
                operation,
                TimelineCatalogEventKind::BranchCreated {
                    branch,
                    parent: self.timeline.cursor,
                    head: self.timeline.cursor,
                },
                &mut transaction,
            )?;
        }
        let outcome = self.store.commit(transaction)?;
        let value = outcome
            .refs
            .get(&key)
            .and_then(|value| *value)
            .ok_or(CoordinatorError::MissingRef)?;
        if let Some(catalog) = outcome.catalogs.get(&CatalogRefKey::new(self.execution)) {
            self.catalog_revision = catalog.map(|value| value.revision);
        }
        self.branch_key = key;
        self.branch_revision = value.revision;
        self.timeline.selected_branch = branch;
        Ok(value)
    }

    pub fn save(
        &mut self,
        owner: RefName,
        slot: ArchiveRefName,
        expected: Option<RefRevision>,
        operation: TimelineOperationId,
        observed_at: u64,
    ) -> Result<SaveResult, CoordinatorError> {
        let key = RefKey::save(owner, slot.clone());
        let actual = self.store.read_ref(&key)?;
        let kind = match actual {
            None => TimelineCatalogEventKind::SaveCreated {
                name: slot.clone(),
                commit: self.timeline.cursor,
            },
            Some(value) => TimelineCatalogEventKind::SaveUpdated {
                name: slot.clone(),
                previous_commit: value.commit,
                next_commit: self.timeline.cursor,
            },
        };
        let mut transaction = CommitTransaction {
            refs: vec![RefMutation {
                key: key.clone(),
                expected,
                next: Some(self.timeline.cursor),
            }],
            observed_at,
            ..CommitTransaction::default()
        };
        if let TimelineRecordingMode::Complete { coverage } = self.recording {
            append_catalog(
                &self.store,
                self.execution,
                self.catalog_revision,
                coverage,
                operation,
                kind,
                &mut transaction,
            )?;
        }
        let outcome = self.store.commit(transaction)?;
        if let Some(catalog) = outcome.catalogs.get(&CatalogRefKey::new(self.execution)) {
            self.catalog_revision = catalog.map(|value| value.revision);
        }
        Ok(SaveResult::Saved(
            outcome
                .refs
                .get(&key)
                .and_then(|value| *value)
                .ok_or(CoordinatorError::MissingRef)?,
        ))
    }

    pub fn delete_save(
        &mut self,
        owner: RefName,
        slot: ArchiveRefName,
        expected: RefRevision,
        operation: TimelineOperationId,
        observed_at: u64,
    ) -> Result<(), CoordinatorError> {
        let key = RefKey::save(owner, slot.clone());
        let actual = self
            .store
            .read_ref(&key)?
            .ok_or(CoordinatorError::MissingRef)?;
        let mut transaction = CommitTransaction {
            refs: vec![RefMutation {
                key,
                expected: Some(expected),
                next: None,
            }],
            observed_at,
            ..CommitTransaction::default()
        };
        if let TimelineRecordingMode::Complete { coverage } = self.recording {
            append_catalog(
                &self.store,
                self.execution,
                self.catalog_revision,
                coverage,
                operation,
                TimelineCatalogEventKind::SaveDeleted {
                    name: slot,
                    previous_commit: actual.commit,
                },
                &mut transaction,
            )?;
        }
        let outcome = self.store.commit(transaction)?;
        if let Some(catalog) = outcome.catalogs.get(&CatalogRefKey::new(self.execution)) {
            self.catalog_revision = catalog.map(|value| value.revision);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rename_save(
        &mut self,
        owner: RefName,
        from: ArchiveRefName,
        to: ArchiveRefName,
        expected_from: RefRevision,
        operation: TimelineOperationId,
        observed_at: u64,
    ) -> Result<RefValue, CoordinatorError> {
        let from_key = RefKey::save(owner.clone(), from.clone());
        let to_key = RefKey::save(owner, to.clone());
        let source = self
            .store
            .read_ref(&from_key)?
            .ok_or(CoordinatorError::MissingRef)?;
        let mut transaction = CommitTransaction {
            refs: vec![
                RefMutation {
                    key: from_key,
                    expected: Some(expected_from),
                    next: None,
                },
                RefMutation {
                    key: to_key.clone(),
                    expected: None,
                    next: Some(source.commit),
                },
            ],
            observed_at,
            ..CommitTransaction::default()
        };
        if let TimelineRecordingMode::Complete { coverage } = self.recording {
            append_catalog(
                &self.store,
                self.execution,
                self.catalog_revision,
                coverage,
                operation,
                TimelineCatalogEventKind::SaveRenamed { from, to },
                &mut transaction,
            )?;
        }
        let outcome = self.store.commit(transaction)?;
        self.update_catalog_revision(&outcome);
        outcome
            .refs
            .get(&to_key)
            .and_then(|value| *value)
            .ok_or(CoordinatorError::MissingRef)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn copy_save(
        &mut self,
        owner: RefName,
        from: ArchiveRefName,
        to: ArchiveRefName,
        operation: TimelineOperationId,
        observed_at: u64,
    ) -> Result<RefValue, CoordinatorError> {
        let source = self
            .store
            .read_ref(&RefKey::save(owner.clone(), from))?
            .ok_or(CoordinatorError::MissingRef)?;
        let to_key = RefKey::save(owner, to.clone());
        let mut transaction = CommitTransaction {
            refs: vec![RefMutation {
                key: to_key.clone(),
                expected: None,
                next: Some(source.commit),
            }],
            observed_at,
            ..CommitTransaction::default()
        };
        if let TimelineRecordingMode::Complete { coverage } = self.recording {
            append_catalog(
                &self.store,
                self.execution,
                self.catalog_revision,
                coverage,
                operation,
                TimelineCatalogEventKind::SaveCreated {
                    name: to,
                    commit: source.commit,
                },
                &mut transaction,
            )?;
        }
        let outcome = self.store.commit(transaction)?;
        self.update_catalog_revision(&outcome);
        outcome
            .refs
            .get(&to_key)
            .and_then(|value| *value)
            .ok_or(CoordinatorError::MissingRef)
    }

    pub fn bookmark(
        &mut self,
        owner: RefName,
        name: ArchiveRefName,
        operation: TimelineOperationId,
        observed_at: u64,
    ) -> Result<RefValue, CoordinatorError> {
        let key = RefKey::bookmark(owner, name.clone());
        let mut transaction = CommitTransaction {
            refs: vec![RefMutation {
                key: key.clone(),
                expected: None,
                next: Some(self.timeline.cursor),
            }],
            observed_at,
            ..CommitTransaction::default()
        };
        if let TimelineRecordingMode::Complete { coverage } = self.recording {
            append_catalog(
                &self.store,
                self.execution,
                self.catalog_revision,
                coverage,
                operation,
                TimelineCatalogEventKind::BookmarkCreated {
                    name,
                    commit: self.timeline.cursor,
                },
                &mut transaction,
            )?;
        }
        let outcome = self.store.commit(transaction)?;
        if let Some(catalog) = outcome.catalogs.get(&CatalogRefKey::new(self.execution)) {
            self.catalog_revision = catalog.map(|value| value.revision);
        }
        outcome
            .refs
            .get(&key)
            .and_then(|value| *value)
            .ok_or(CoordinatorError::MissingRef)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rename_bookmark(
        &mut self,
        owner: RefName,
        from: ArchiveRefName,
        to: ArchiveRefName,
        expected_from: RefRevision,
        operation: TimelineOperationId,
        observed_at: u64,
    ) -> Result<RefValue, CoordinatorError> {
        let from_key = RefKey::bookmark(owner.clone(), from.clone());
        let to_key = RefKey::bookmark(owner, to.clone());
        let source = self
            .store
            .read_ref(&from_key)?
            .ok_or(CoordinatorError::MissingRef)?;
        let mut transaction = CommitTransaction {
            refs: vec![
                RefMutation {
                    key: from_key,
                    expected: Some(expected_from),
                    next: None,
                },
                RefMutation {
                    key: to_key.clone(),
                    expected: None,
                    next: Some(source.commit),
                },
            ],
            observed_at,
            ..CommitTransaction::default()
        };
        if let TimelineRecordingMode::Complete { coverage } = self.recording {
            append_catalog(
                &self.store,
                self.execution,
                self.catalog_revision,
                coverage,
                operation,
                TimelineCatalogEventKind::BookmarkRenamed { from, to },
                &mut transaction,
            )?;
        }
        let outcome = self.store.commit(transaction)?;
        self.update_catalog_revision(&outcome);
        outcome
            .refs
            .get(&to_key)
            .and_then(|value| *value)
            .ok_or(CoordinatorError::MissingRef)
    }

    pub fn delete_bookmark(
        &mut self,
        owner: RefName,
        name: ArchiveRefName,
        expected: RefRevision,
        operation: TimelineOperationId,
        observed_at: u64,
    ) -> Result<(), CoordinatorError> {
        let key = RefKey::bookmark(owner, name.clone());
        let actual = self
            .store
            .read_ref(&key)?
            .ok_or(CoordinatorError::MissingRef)?;
        let mut transaction = CommitTransaction {
            refs: vec![RefMutation {
                key,
                expected: Some(expected),
                next: None,
            }],
            observed_at,
            ..CommitTransaction::default()
        };
        if let TimelineRecordingMode::Complete { coverage } = self.recording {
            append_catalog(
                &self.store,
                self.execution,
                self.catalog_revision,
                coverage,
                operation,
                TimelineCatalogEventKind::BookmarkDeleted {
                    name,
                    previous_commit: actual.commit,
                },
                &mut transaction,
            )?;
        }
        let outcome = self.store.commit(transaction)?;
        self.update_catalog_revision(&outcome);
        Ok(())
    }

    pub fn delete_branch(
        &mut self,
        branch: BranchId,
        expected: RefRevision,
        operation: TimelineOperationId,
        observed_at: u64,
    ) -> Result<(), CoordinatorError> {
        if branch == self.timeline.selected_branch {
            return Err(CoordinatorError::Commit(
                "cannot delete the selected branch".to_owned(),
            ));
        }
        let key = RefKey::branch(self.execution, branch)?;
        let actual = self
            .store
            .read_ref(&key)?
            .ok_or(CoordinatorError::MissingRef)?;
        let mut transaction = CommitTransaction {
            refs: vec![RefMutation {
                key,
                expected: Some(expected),
                next: None,
            }],
            observed_at,
            ..CommitTransaction::default()
        };
        if let TimelineRecordingMode::Complete { coverage } = self.recording {
            append_catalog(
                &self.store,
                self.execution,
                self.catalog_revision,
                coverage,
                operation,
                TimelineCatalogEventKind::BranchDeleted {
                    branch,
                    previous_head: actual.commit,
                },
                &mut transaction,
            )?;
        }
        let outcome = self.store.commit(transaction)?;
        self.update_catalog_revision(&outcome);
        Ok(())
    }

    pub fn load_save(
        &mut self,
        owner: RefName,
        slot: RefName,
        observed_at: u64,
    ) -> Result<LoadedCommit, CoordinatorError> {
        let value = self
            .store
            .read_ref(&RefKey::save(owner, slot))?
            .ok_or(CoordinatorError::MissingRef)?;
        let loaded = load_commit(&self.store, value.commit, &self.program)?;
        let outcome = self.store.commit(CommitTransaction {
            refs: vec![RefMutation {
                key: self.session_key.clone(),
                expected: Some(self.active_revision),
                next: Some(value.commit),
            }],
            observed_at,
            ..CommitTransaction::default()
        })?;
        self.active_revision = outcome
            .refs
            .get(&self.session_key)
            .and_then(|value| *value)
            .map(|value| value.revision)
            .ok_or(CoordinatorError::MissingRef)?;
        self.timeline.cursor = value.commit;
        self.state = Arc::new(loaded.state.clone());
        Ok(loaded)
    }

    pub fn enable_complete_recording(
        &mut self,
        operation: TimelineOperationId,
        observed_at: u64,
    ) -> Result<TimelineCoverage, CoordinatorError> {
        if matches!(self.recording, TimelineRecordingMode::Complete { .. }) {
            return Err(CoordinatorError::AlreadyComplete);
        }
        let coverage = TimelineCoverage::FromBaseline {
            baseline: self.timeline.cursor,
        };
        let mut initial_refs = Vec::new();
        for (key, value) in self.store.list_refs()? {
            let Ok(commit) = read_commit(&self.store, value.commit) else {
                continue;
            };
            if commit.execution != self.execution {
                continue;
            }
            match key.namespace() {
                crate::RefNamespace::Branch => {
                    if let Ok(bytes) = hex::decode(key.name().as_str())
                        && let Ok(bytes) = <[u8; 16]>::try_from(bytes)
                    {
                        initial_refs.push(ArchivedRefSnapshot::Branch(ArchivedBranchRef {
                            branch: BranchId::from_bytes(bytes),
                            head: value.commit,
                        }));
                    }
                }
                crate::RefNamespace::Save => {
                    initial_refs.push(ArchivedRefSnapshot::Save(ArchivedSaveRef {
                        name: key.name().clone(),
                        commit: value.commit,
                    }));
                }
                crate::RefNamespace::Bookmark => {
                    initial_refs.push(ArchivedRefSnapshot::Bookmark(ArchivedBookmark {
                        name: key.name().clone(),
                        commit: value.commit,
                    }));
                }
                crate::RefNamespace::Active | crate::RefNamespace::Temporary => {}
            }
        }
        initial_refs.sort();
        initial_refs.dedup();
        let event = TimelineCatalogEventV1 {
            execution: self.execution,
            previous: None,
            operation,
            kind: TimelineCatalogEventKind::RecordingStarted {
                baseline: self.timeline.cursor,
                initial_refs,
            },
        };
        let object = event
            .to_object()
            .map_err(|error| CoordinatorError::Catalog(error.to_string()))?;
        let id = TimelineCatalogEventId::from_bytes(*object.id().as_bytes());
        let outcome = self.store.commit(CommitTransaction {
            objects: vec![object],
            catalogs: vec![CatalogMutation {
                key: CatalogRefKey::new(self.execution),
                expected: None,
                next: Some(id),
                coverage,
            }],
            observed_at,
            ..CommitTransaction::default()
        })?;
        self.catalog_revision = outcome
            .catalogs
            .get(&CatalogRefKey::new(self.execution))
            .and_then(|value| *value)
            .map(|value| value.revision);
        self.recording = TimelineRecordingMode::Complete { coverage };
        Ok(coverage)
    }

    pub fn delete_complete_recording(&mut self, observed_at: u64) -> Result<(), CoordinatorError> {
        let TimelineRecordingMode::Complete { coverage } = self.recording else {
            return Err(CoordinatorError::NotComplete);
        };
        self.store.commit(CommitTransaction {
            catalogs: vec![CatalogMutation {
                key: CatalogRefKey::new(self.execution),
                expected: self.catalog_revision,
                next: None,
                coverage,
            }],
            observed_at,
            ..CommitTransaction::default()
        })?;
        self.catalog_revision = None;
        self.recording = TimelineRecordingMode::Standard;
        Ok(())
    }

    pub fn seal_complete_recording(
        &mut self,
        archive_name: RefName,
        observed_at: u64,
    ) -> Result<TimelineArchiveManifestId, CoordinatorError> {
        let TimelineRecordingMode::Complete { coverage } = self.recording else {
            return Err(CoordinatorError::NotComplete);
        };
        let bundle = TimelineArchiveBundle::export(
            &self.store,
            self.execution,
            Some(self.timeline),
            &BTreeSet::new(),
        )
        .map_err(|error| CoordinatorError::Commit(error.to_string()))?;
        let manifest_object = bundle
            .manifest
            .to_object()
            .map_err(|error| CoordinatorError::Commit(error.to_string()))?;
        let manifest_id = TimelineArchiveManifestId::from_bytes(*manifest_object.id().as_bytes());
        self.store.commit(CommitTransaction {
            objects: vec![manifest_object],
            catalogs: vec![CatalogMutation {
                key: CatalogRefKey::new(self.execution),
                expected: self.catalog_revision,
                next: None,
                coverage,
            }],
            archives: vec![ArchiveMutation {
                key: TimelineArchiveRefKey::new(self.execution, archive_name),
                expected: None,
                next: Some(manifest_id),
            }],
            observed_at,
            ..CommitTransaction::default()
        })?;
        self.catalog_revision = None;
        self.recording = TimelineRecordingMode::Standard;
        Ok(manifest_id)
    }

    pub fn delete_archive_root(
        &mut self,
        archive_name: RefName,
        expected: RefRevision,
        observed_at: u64,
    ) -> Result<(), CoordinatorError> {
        self.store.commit(CommitTransaction {
            archives: vec![ArchiveMutation {
                key: TimelineArchiveRefKey::new(self.execution, archive_name),
                expected: Some(expected),
                next: None,
            }],
            observed_at,
            ..CommitTransaction::default()
        })?;
        Ok(())
    }

    fn update_catalog_revision(&mut self, outcome: &crate::CommitOutcome) {
        if let Some(catalog) = outcome.catalogs.get(&CatalogRefKey::new(self.execution)) {
            self.catalog_revision = catalog.map(|value| value.revision);
        }
    }
}

pub fn read_commit(store: &impl SaveStore, id: CommitId) -> Result<CommitV1, CoordinatorError> {
    let object = store
        .get_object(ObjectId::from_bytes(*id.as_bytes()))?
        .ok_or(StoreError::MissingObject(ObjectId::from_bytes(
            *id.as_bytes(),
        )))?;
    if object.kind() != ObjectKind::Commit {
        return Err(StoreError::ObjectKind(object.id()).into());
    }
    CommitV1::decode(object.payload()).map_err(|error| CoordinatorError::Commit(error.to_string()))
}

pub fn load_commit(
    store: &impl SaveStore,
    id: CommitId,
    program: &CheckedProgram,
) -> Result<LoadedCommit, CoordinatorError> {
    let commit = read_commit(store, id)?;
    if commit.program != program.artifact_id() {
        return Err(CoordinatorError::IncompatibleCommit);
    }
    let snapshot_object = store
        .get_object(ObjectId::from_bytes(*commit.snapshot.as_bytes()))?
        .ok_or(StoreError::MissingObject(ObjectId::from_bytes(
            *commit.snapshot.as_bytes(),
        )))?;
    if snapshot_object.kind() != ObjectKind::Snapshot {
        return Err(StoreError::ObjectKind(snapshot_object.id()).into());
    }
    let state = restore_snapshot(
        snapshot_object.bytes(),
        program,
        &SnapshotLoadLimits::default(),
    )
    .map_err(|error| CoordinatorError::Snapshot(error.to_string()))?;
    if state.execution_id != commit.execution
        || state.program_artifact_id != commit.program
        || state.turn != commit.turn
    {
        return Err(CoordinatorError::IncompatibleCommit);
    }
    validate_ancestor_closure(store, id, commit.execution, commit.program)?;
    Ok(LoadedCommit { id, commit, state })
}

fn validate_ancestor_closure(
    store: &impl SaveStore,
    start: CommitId,
    execution: ExecutionId,
    program: ProgramArtifactId,
) -> Result<(), CoordinatorError> {
    let mut seen = BTreeSet::new();
    let mut current = Some(start);
    while let Some(id) = current {
        if !seen.insert(id) {
            return Err(CoordinatorError::Commit("Commit parent cycle".to_owned()));
        }
        if seen.len() > 1_000_000 {
            return Err(StoreError::Limit("Commit ancestry").into());
        }
        let commit = read_commit(store, id)?;
        if commit.execution != execution || commit.program != program {
            return Err(CoordinatorError::IncompatibleCommit);
        }
        current = commit.parent;
    }
    Ok(())
}

fn ensure_ancestor(
    store: &impl SaveStore,
    ancestor: CommitId,
    descendant: CommitId,
) -> Result<(), CoordinatorError> {
    path_from_ancestor(store, ancestor, descendant).map(|_| ())
}

fn path_from_ancestor(
    store: &impl SaveStore,
    ancestor: CommitId,
    descendant: CommitId,
) -> Result<Vec<CommitId>, CoordinatorError> {
    let mut reverse = Vec::new();
    let mut current = descendant;
    let mut seen = BTreeSet::new();
    while current != ancestor {
        if !seen.insert(current) || seen.len() > 1_000_000 {
            return Err(CoordinatorError::NotOnSelectedBranch);
        }
        reverse.push(current);
        current = read_commit(store, current)?
            .parent
            .ok_or(CoordinatorError::NotOnSelectedBranch)?;
    }
    reverse.reverse();
    Ok(reverse)
}

fn run_transition(
    program: Arc<CheckedProgram>,
    state: Arc<RuntimeStateV0>,
    input: CheckedRuntimeInput,
    limits: MacrostepLimits,
) -> Result<narrata_core::runtime::TransitionDraft, CoordinatorError> {
    let mut outcome =
        begin_transition(program, state, input, limits)?.run_slice(SliceBudget::unlimited());
    loop {
        match outcome {
            SliceOutcome::Yielded { runner, .. } => {
                outcome = runner.run_slice(SliceBudget::unlimited());
            }
            SliceOutcome::Completed(draft) => return Ok(draft),
            SliceOutcome::Faulted(error) => return Err(error.into()),
        }
    }
}

fn checked_existing_object(
    bytes: &[u8],
    kind: ObjectKind,
    schema: u16,
) -> Result<CheckedObject, CoordinatorError> {
    CheckedObject::from_bytes(bytes, kind, schema, 512 * 1024 * 1024)
        .map_err(|error| CoordinatorError::Object(error.to_string()))
}

fn result_from_state(state: &RuntimeStateV0) -> Result<DraftResult, CoordinatorError> {
    match &state.status {
        narrata_core::runtime::RuntimeStatusV0::Awaiting { pending, .. } => match pending {
            narrata_core::runtime::PendingInteractionV0::Say {
                interaction_id,
                speaker,
                text,
                ..
            } => Ok(DraftResult::AwaitSay(narrata_core::runtime::SayView {
                interaction_id: *interaction_id,
                speaker: speaker.clone(),
                text: text.clone(),
            })),
            narrata_core::runtime::PendingInteractionV0::Choice {
                interaction_id,
                prompt,
                offered,
                ..
            } => Ok(DraftResult::AwaitChoice(
                narrata_core::runtime::ChoiceView {
                    interaction_id: *interaction_id,
                    prompt: prompt.clone(),
                    choices: offered
                        .iter()
                        .map(|choice| narrata_core::runtime::ChoiceViewItem {
                            id: choice.id,
                            label: choice.label.clone(),
                        })
                        .collect(),
                },
            )),
        },
        narrata_core::runtime::RuntimeStatusV0::Finished { result, .. } => {
            Ok(DraftResult::Finished(result.clone()))
        }
        narrata_core::runtime::RuntimeStatusV0::Ready { .. } => Err(CoordinatorError::Commit(
            "committed transition ended outside an interaction safe point".to_owned(),
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn append_catalog(
    store: &impl SaveStore,
    execution: ExecutionId,
    expected: Option<RefRevision>,
    coverage: TimelineCoverage,
    operation: TimelineOperationId,
    kind: TimelineCatalogEventKind,
    transaction: &mut CommitTransaction,
) -> Result<(), CoordinatorError> {
    let key = CatalogRefKey::new(execution);
    let actual = store
        .read_catalog_head(&key)?
        .ok_or(CoordinatorError::NotComplete)?;
    if Some(actual.revision) != expected || actual.coverage != coverage {
        return Err(StoreError::from(crate::CatalogConflict {
            key: key.storage_key(),
            expected,
            actual: Some(actual),
            proposed: None,
        })
        .into());
    }
    let event = TimelineCatalogEventV1 {
        execution,
        previous: Some(actual.event),
        operation,
        kind,
    };
    let object = event
        .to_object()
        .map_err(|error| CoordinatorError::Catalog(error.to_string()))?;
    if object.schema() != CATALOG_EVENT_SCHEMA_V1 {
        return Err(CoordinatorError::Catalog(
            "Catalog schema mismatch".to_owned(),
        ));
    }
    let event_id = TimelineCatalogEventId::from_bytes(*object.id().as_bytes());
    transaction.objects.push(object);
    transaction.catalogs.push(CatalogMutation {
        key,
        expected,
        next: Some(event_id),
        coverage,
    });
    Ok(())
}

fn deterministic_branch(parent: CommitId, payload: &[u8; 32]) -> BranchId {
    let mut hasher = Sha256::new();
    hasher.update(b"NARRATA-BRANCH\0");
    hasher.update(parent.as_bytes());
    hasher.update(payload);
    let digest = hasher.finalize();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    BranchId::from_bytes(bytes)
}

fn deterministic_operation(domain: &[u8], source: &[u8]) -> TimelineOperationId {
    let mut hasher = Sha256::new();
    hasher.update(b"NARRATA-TIMELINE-OP\0");
    hasher.update(domain);
    hasher.update(source);
    let digest = hasher.finalize();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    TimelineOperationId::from_bytes(bytes)
}

#[allow(dead_code)]
fn find_program_object(
    store: &impl SaveStore,
    id: ProgramArtifactId,
) -> Result<CheckedObject, CoordinatorError> {
    store
        .list_objects()?
        .into_iter()
        .filter(|object| object.kind() == ObjectKind::Program)
        .find_map(|object| {
            load_program(object.bytes(), &ProgramLoadLimits::default())
                .ok()
                .filter(|program| program.artifact_id() == id)
                .map(|_| object)
        })
        .ok_or(CoordinatorError::IncompatibleCommit)
}

#[allow(dead_code)]
fn _catalog_head_is_copy(_: CatalogHeadRefValue) {}
