use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use narrata_core::{
    CapabilityId, CapabilityVersion, CheckedProgram, ChoiceId, CommitId, EffectId,
    EffectRequestDigest, EventTypeId, ExecutionId, InputId, InteractionId, ObjectId,
    ProgramArtifactId, ReceiptId, SnapshotId, StateDigest, Value, builtin_host_capabilities,
    codec::{ObjectKind, decode_canonical_value, encode_canonical_value},
    effect::{EffectResponseV0, HostCapabilities, negotiate_capabilities},
    limits::{DecodeLimits, MacrostepLimits, ProgramLoadLimits, SnapshotLoadLimits},
    program::load_program,
    runtime::{
        CheckedRuntimeInput, ChoiceView, ChoiceViewItem, DraftResult, PendingInteractionV0,
        RuntimeStateV0, RuntimeStatusV0, SayView, SliceBudget, SliceOutcome, TransitionDraft,
        TransitionRunner, begin_transition_with_parent_commit, new_execution,
    },
    snapshot::{export_snapshot, state_digest},
    version::PROTOCOL_V1,
};
use narrata_store::{
    ArchivedBranchRef, ArchivedRefSnapshot, BranchId, BundleLimits, CatalogMutation, CatalogRefKey,
    CheckedObject, CheckpointBundle, CommitCauseV1, CommitOutcome, CommitTransaction, CommitV1,
    InputRecord, MemoryStore, RefKey, RefMutation, RefName, RefRevision, STORED_RECEIPT_SCHEMA_V1,
    SaveStore, TimelineArchiveBundle, TimelineCatalogEventKind, TimelineCatalogEventV1,
    TimelineCoverage, TimelineImportMapping, TimelineOperationId, TimelineSession,
    TransitionReceiptV1, load_commit, timeline_branch,
};
use prost::Message;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::dto::{self, request, response, runtime_input};

pub const PROTOCOL_ABI_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProtocolLimits {
    pub max_message_bytes: u64,
    pub max_slice_work: u64,
    pub max_sessions: u64,
    pub program: ProgramLoadLimits,
    pub snapshot: SnapshotLoadLimits,
    pub macrostep: MacrostepLimits,
    pub bundle: BundleLimits,
}

impl Default for ProtocolLimits {
    fn default() -> Self {
        Self {
            max_message_bytes: 16 * 1024 * 1024,
            max_slice_work: 100_000,
            max_sessions: 1_024,
            program: ProgramLoadLimits::default(),
            snapshot: SnapshotLoadLimits::default(),
            macrostep: MacrostepLimits::default(),
            bundle: BundleLimits::default(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ProtocolBoundaryError {
    #[error("protocol message exceeds configured byte limit")]
    Oversize,
    #[error("invalid Protobuf message: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("encoded protocol response exceeds configured byte limit")]
    ResponseOversize,
}

struct Session {
    program: Arc<CheckedProgram>,
    execution: ExecutionId,
    store: MemoryStore,
    commit: CommitId,
    state: Arc<RuntimeStateV0>,
    pending: Option<TransitionRunner>,
    branch: BranchId,
    branch_key: RefKey,
    active_key: RefKey,
    branch_revision: RefRevision,
    active_revision: RefRevision,
    catalog_revision: RefRevision,
    coverage: TimelineCoverage,
}

pub struct ProtocolEngine {
    limits: ProtocolLimits,
    programs: BTreeMap<ProgramArtifactId, Arc<CheckedProgram>>,
    sessions: BTreeMap<u64, Session>,
    next_session: u64,
}

impl Default for ProtocolEngine {
    fn default() -> Self {
        Self::new(ProtocolLimits::default())
    }
}

impl ProtocolEngine {
    pub fn new(limits: ProtocolLimits) -> Self {
        Self {
            limits,
            programs: BTreeMap::new(),
            sessions: BTreeMap::new(),
            next_session: 1,
        }
    }

    pub const fn limits(&self) -> ProtocolLimits {
        self.limits
    }

    pub fn handle_bytes(&mut self, bytes: &[u8]) -> Result<Vec<u8>, ProtocolBoundaryError> {
        if bytes.len() as u64 > self.limits.max_message_bytes {
            return Err(ProtocolBoundaryError::Oversize);
        }
        let request = dto::Request::decode(bytes)?;
        let response = self.handle(request);
        if response.encoded_len() as u64 > self.limits.max_message_bytes {
            return Err(ProtocolBoundaryError::ResponseOversize);
        }
        Ok(response.encode_to_vec())
    }

    pub fn handle(&mut self, request: dto::Request) -> dto::Response {
        let request_id = request.request_id;
        if request.protocol_version != u32::from(PROTOCOL_V1.get()) {
            return diagnostic_response(
                request_id,
                ProtocolDiagnostic::incompatible(format!(
                    "unsupported protocol version {}",
                    request.protocol_version
                )),
            );
        }
        let body = match request.body {
            Some(request::Body::EngineCreate(value)) => self.engine_create(value),
            Some(request::Body::ProgramLoad(value)) => self.program_load(value),
            Some(request::Body::SessionCreate(value)) => self.session_create(value),
            Some(request::Body::SessionLoad(value)) => {
                self.session_load(value.artifact_id, value.checkpoint_bundle)
            }
            Some(request::Body::Dispatch(value)) => self.dispatch(value),
            Some(request::Body::ContinueSlice(value)) => self.continue_slice(value),
            Some(request::Body::CheckpointExport(value)) => self.checkpoint_export(value),
            Some(request::Body::CheckpointImport(value)) => {
                self.session_load(value.artifact_id, value.bundle)
            }
            Some(request::Body::CapabilityNegotiation(value)) => self.capability_negotiation(value),
            Some(request::Body::TimelineArchiveExport(value)) => {
                self.timeline_archive_export(value)
            }
            Some(request::Body::TimelineArchiveImport(value)) => {
                self.timeline_archive_import(value)
            }
            None => Err(ProtocolDiagnostic::invalid("request body is missing")),
        };
        match body {
            Ok(body) => dto::Response {
                protocol_version: u32::from(PROTOCOL_V1.get()),
                request_id,
                body: Some(body),
            },
            Err(error) => diagnostic_response(request_id, error),
        }
    }

    fn engine_create(
        &mut self,
        request: dto::EngineCreate,
    ) -> Result<response::Body, ProtocolDiagnostic> {
        if request.max_message_bytes != 0 {
            self.limits.max_message_bytes = self
                .limits
                .max_message_bytes
                .min(request.max_message_bytes.max(1_024));
        }
        if request.max_slice_work != 0 {
            self.limits.max_slice_work = self
                .limits
                .max_slice_work
                .min(request.max_slice_work.max(1));
        }
        Ok(response::Body::EngineCreated(dto::EngineCreated {
            abi_version: PROTOCOL_ABI_VERSION,
            max_message_bytes: self.limits.max_message_bytes,
        }))
    }

    fn program_load(
        &mut self,
        request: dto::ProgramLoad,
    ) -> Result<response::Body, ProtocolDiagnostic> {
        if request.artifact.len() as u64 > self.limits.program.decode.max_envelope_bytes {
            return Err(ProtocolDiagnostic::limit("Program Artifact bytes"));
        }
        let program = load_program(&request.artifact, &self.limits.program)
            .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        let id = program.artifact_id();
        self.programs.insert(id, program);
        Ok(response::Body::ProgramLoaded(dto::ProgramLoaded {
            artifact_id: id.as_bytes().to_vec(),
        }))
    }

    fn session_create(
        &mut self,
        request: dto::SessionCreate,
    ) -> Result<response::Body, ProtocolDiagnostic> {
        self.ensure_session_capacity()?;
        let artifact = ProgramArtifactId::from_bytes(id32(&request.artifact_id, "Artifact ID")?);
        let execution = ExecutionId::from_bytes(id16(&request.execution_id, "Execution ID")?);
        let program = self
            .programs
            .get(&artifact)
            .cloned()
            .ok_or_else(|| ProtocolDiagnostic::missing("Program Artifact is not loaded"))?;
        let host = builtin_host_capabilities()
            .map_err(|error| ProtocolDiagnostic::capability(error.to_string()))?;
        negotiate_capabilities(&program.artifact().capabilities, &host)
            .map_err(|errors| ProtocolDiagnostic::capability(format!("{errors:?}")))?;
        let state = Arc::new(
            new_execution(&program, execution)
                .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?,
        );
        let mut store = MemoryStore::new();
        let program_object = checked_object(
            &narrata_core::program::encode_program_artifact(program.artifact()),
            ObjectKind::Program,
            0,
            self.limits.program.decode.max_envelope_bytes,
        )?;
        let snapshot_bytes = export_snapshot(&state)
            .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        let snapshot_object = checked_object(
            &snapshot_bytes,
            ObjectKind::Snapshot,
            0,
            self.limits.snapshot.decode.max_envelope_bytes,
        )?;
        let snapshot = SnapshotId::from_bytes(*snapshot_object.id().as_bytes());
        let commit_object = CommitV1 {
            parent: None,
            execution,
            program: artifact,
            snapshot,
            cause: CommitCauseV1::Genesis,
            ledger_fence: 0,
            turn: state.turn,
        }
        .to_object();
        let commit = CommitId::from_bytes(*commit_object.id().as_bytes());
        let branch = BranchId::from_bytes(*execution.as_bytes());
        let branch_key = timeline_branch(execution, branch);
        let active_key = protocol_active_key()?;
        let coverage = TimelineCoverage::FromBaseline { baseline: commit };
        let catalog_event = recording_started(execution, branch, commit)?;
        let catalog_event_id =
            narrata_core::TimelineCatalogEventId::from_bytes(*catalog_event.id().as_bytes());
        let outcome = store
            .commit(CommitTransaction {
                objects: vec![
                    program_object,
                    snapshot_object,
                    commit_object,
                    catalog_event,
                ],
                refs: vec![
                    RefMutation {
                        key: branch_key.clone(),
                        expected: None,
                        next: Some(commit),
                    },
                    RefMutation {
                        key: active_key.clone(),
                        expected: None,
                        next: Some(commit),
                    },
                ],
                catalogs: vec![CatalogMutation {
                    key: CatalogRefKey::new(execution),
                    expected: None,
                    next: Some(catalog_event_id),
                    coverage,
                }],
                ..CommitTransaction::default()
            })
            .map_err(store_diagnostic)?;
        let session = self.allocate_session(Session {
            program,
            execution,
            store,
            commit,
            state,
            pending: None,
            branch,
            branch_revision: outcome_ref_revision(&outcome, &branch_key)?,
            active_revision: outcome_ref_revision(&outcome, &active_key)?,
            catalog_revision: outcome_catalog_revision(&outcome, execution)?,
            branch_key,
            active_key,
            coverage,
        })?;
        Ok(response::Body::SessionCreated(dto::SessionCreated {
            session,
            commit_id: commit.as_bytes().to_vec(),
            snapshot: snapshot_bytes,
        }))
    }

    fn session_load(
        &mut self,
        artifact_bytes: Vec<u8>,
        bundle_bytes: Vec<u8>,
    ) -> Result<response::Body, ProtocolDiagnostic> {
        self.ensure_session_capacity()?;
        let artifact = ProgramArtifactId::from_bytes(id32(&artifact_bytes, "Artifact ID")?);
        let program = self
            .programs
            .get(&artifact)
            .cloned()
            .ok_or_else(|| ProtocolDiagnostic::missing("Program Artifact is not loaded"))?;
        let bundle = CheckpointBundle::from_bytes(&bundle_bytes, self.limits.bundle)
            .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        let root = bundle.manifest.root;
        let mut store = MemoryStore::new();
        let target = protocol_active_key()?;
        let active = bundle
            .import(&mut store, target.clone(), None, 0)
            .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        let loaded = load_commit(&store, root, &program)
            .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        let execution = loaded.commit.execution;
        let snapshot = store
            .get_object(ObjectId::from_bytes(*loaded.commit.snapshot.as_bytes()))
            .map_err(store_diagnostic)?
            .ok_or_else(|| ProtocolDiagnostic::missing("Snapshot object is missing"))?
            .bytes()
            .to_vec();
        let branch = BranchId::from_bytes(*execution.as_bytes());
        let branch_key = timeline_branch(execution, branch);
        let coverage = TimelineCoverage::FromBaseline { baseline: root };
        let catalog_event = recording_started(execution, branch, root)?;
        let catalog_event_id =
            narrata_core::TimelineCatalogEventId::from_bytes(*catalog_event.id().as_bytes());
        let outcome = store
            .commit(CommitTransaction {
                objects: vec![catalog_event],
                refs: vec![RefMutation {
                    key: branch_key.clone(),
                    expected: None,
                    next: Some(root),
                }],
                catalogs: vec![CatalogMutation {
                    key: CatalogRefKey::new(execution),
                    expected: None,
                    next: Some(catalog_event_id),
                    coverage,
                }],
                ..CommitTransaction::default()
            })
            .map_err(store_diagnostic)?;
        let session = self.allocate_session(Session {
            program,
            execution,
            store,
            commit: root,
            state: Arc::new(loaded.state),
            pending: None,
            branch,
            branch_revision: outcome_ref_revision(&outcome, &branch_key)?,
            branch_key,
            active_key: target,
            active_revision: active.revision,
            catalog_revision: outcome_catalog_revision(&outcome, execution)?,
            coverage,
        })?;
        Ok(response::Body::SessionCreated(dto::SessionCreated {
            session,
            commit_id: root.as_bytes().to_vec(),
            snapshot,
        }))
    }

    fn dispatch(&mut self, request: dto::Dispatch) -> Result<response::Body, ProtocolDiagnostic> {
        let limits = self.limits;
        let session = self
            .sessions
            .get_mut(&request.session)
            .ok_or_else(|| ProtocolDiagnostic::invalid("invalid session handle"))?;
        if session.pending.is_some() {
            return Err(ProtocolDiagnostic::conflict(
                "a transition slice is already in progress",
            ));
        }
        let input = parse_input(
            request
                .input
                .ok_or_else(|| ProtocolDiagnostic::invalid("Runtime input is missing"))?,
            &session.state,
            &session.program,
            &limits.snapshot.decode,
        )?;
        if let Some(existing) = session
            .store
            .read_input(session.execution, input.request_id())
            .map_err(store_diagnostic)?
        {
            if existing.parent != session.commit || existing.payload != input.payload_digest() {
                return Err(ProtocolDiagnostic::conflict(
                    "InputId was committed with another parent or payload",
                ));
            }
            return reuse_committed(request.session, session, existing.commit);
        }
        let runner = begin_transition_with_parent_commit(
            session.program.clone(),
            session.state.clone(),
            session.commit,
            input,
            limits.macrostep,
        )
        .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        run_slice(
            request.session,
            session,
            runner,
            bounded_slice(request.slice_work, limits.max_slice_work)?,
            limits,
        )
    }

    fn continue_slice(
        &mut self,
        request: dto::ContinueSlice,
    ) -> Result<response::Body, ProtocolDiagnostic> {
        let limits = self.limits;
        let session = self
            .sessions
            .get_mut(&request.session)
            .ok_or_else(|| ProtocolDiagnostic::invalid("invalid session handle"))?;
        let runner = session
            .pending
            .take()
            .ok_or_else(|| ProtocolDiagnostic::invalid("no transition slice is pending"))?;
        run_slice(
            request.session,
            session,
            runner,
            bounded_slice(request.slice_work, limits.max_slice_work)?,
            limits,
        )
    }

    fn checkpoint_export(
        &mut self,
        request: dto::CheckpointExport,
    ) -> Result<response::Body, ProtocolDiagnostic> {
        let session = self
            .sessions
            .get(&request.session)
            .ok_or_else(|| ProtocolDiagnostic::invalid("invalid session handle"))?;
        if session.pending.is_some() {
            return Err(ProtocolDiagnostic::conflict(
                "cannot export while a transition slice is in progress",
            ));
        }
        let bytes = CheckpointBundle::export(&session.store, session.commit, &BTreeSet::new())
            .and_then(|bundle| bundle.to_bytes())
            .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        if bytes.len() as u64 > self.limits.max_message_bytes {
            return Err(ProtocolDiagnostic::limit(
                "Checkpoint Bundle response bytes",
            ));
        }
        Ok(response::Body::Bundle(dto::Bundle {
            kind: dto::BundleKind::Checkpoint as i32,
            bytes,
        }))
    }

    fn timeline_archive_export(
        &mut self,
        request: dto::TimelineArchiveExport,
    ) -> Result<response::Body, ProtocolDiagnostic> {
        let session = self
            .sessions
            .get(&request.session)
            .ok_or_else(|| ProtocolDiagnostic::invalid("invalid session handle"))?;
        if session.pending.is_some() {
            return Err(ProtocolDiagnostic::conflict(
                "cannot export while a transition slice is in progress",
            ));
        }
        let bytes = TimelineArchiveBundle::export(
            &session.store,
            session.execution,
            Some(TimelineSession {
                selected_branch: session.branch,
                cursor: session.commit,
            }),
            &BTreeSet::new(),
        )
        .and_then(|bundle| bundle.to_bytes())
        .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        if bytes.len() as u64 > self.limits.max_message_bytes {
            return Err(ProtocolDiagnostic::limit("Timeline Archive response bytes"));
        }
        Ok(response::Body::Bundle(dto::Bundle {
            kind: dto::BundleKind::TimelineArchive as i32,
            bytes,
        }))
    }

    fn timeline_archive_import(
        &mut self,
        request: dto::TimelineArchiveImport,
    ) -> Result<response::Body, ProtocolDiagnostic> {
        self.ensure_session_capacity()?;
        let bundle = TimelineArchiveBundle::from_bytes(&request.bundle, self.limits.bundle)
            .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        let active = bundle
            .manifest
            .active
            .ok_or_else(|| ProtocolDiagnostic::invalid("Timeline Archive has no active session"))?;
        let execution = bundle.manifest.execution;
        let commit_object_id = ObjectId::from_bytes(*active.cursor.as_bytes());
        let commit_object = bundle
            .objects
            .iter()
            .find(|object| object.id() == commit_object_id)
            .ok_or_else(|| ProtocolDiagnostic::missing("active Commit object is missing"))?;
        let commit = CommitV1::decode(commit_object.payload())
            .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        let program = bundle
            .objects
            .iter()
            .filter(|object| object.kind() == ObjectKind::Program)
            .find_map(|object| {
                load_program(object.bytes(), &self.limits.program)
                    .ok()
                    .filter(|program| program.artifact_id() == commit.program)
            })
            .ok_or_else(|| ProtocolDiagnostic::missing("Program Artifact object is missing"))?;
        let program = self
            .programs
            .entry(program.artifact_id())
            .or_insert(program)
            .clone();
        let branch_ids = bundle
            .manifest
            .branch_heads
            .iter()
            .map(|value| (value.branch, value.branch))
            .collect();
        let save_names = bundle
            .manifest
            .save_refs
            .iter()
            .map(|value| (value.name.clone(), value.name.clone()))
            .collect();
        let bookmark_names = bundle
            .manifest
            .bookmarks
            .iter()
            .map(|value| (value.name.clone(), value.name.clone()))
            .collect();
        let active_key = protocol_active_key()?;
        let mapping = TimelineImportMapping {
            archive_name: RefName::new("protocol-import")
                .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?,
            save_owner: RefName::new("protocol")
                .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?,
            bookmark_owner: RefName::new("protocol")
                .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?,
            branch_ids,
            save_names,
            bookmark_names,
            session_name: Some(active_key.owner().clone()),
        };
        let mut store = MemoryStore::new();
        bundle
            .import(&mut store, mapping, 0)
            .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        let loaded = load_commit(&store, active.cursor, &program)
            .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        let snapshot = store
            .get_object(ObjectId::from_bytes(*loaded.commit.snapshot.as_bytes()))
            .map_err(store_diagnostic)?
            .ok_or_else(|| ProtocolDiagnostic::missing("Snapshot object is missing"))?
            .bytes()
            .to_vec();
        let branch_key = timeline_branch(execution, active.selected_branch);
        let branch_revision = required_ref_revision(&store, &branch_key)?;
        let active_revision = required_ref_revision(&store, &active_key)?;
        let catalog = store
            .read_catalog_head(&CatalogRefKey::new(execution))
            .map_err(store_diagnostic)?
            .ok_or_else(|| ProtocolDiagnostic::missing("Catalog head is missing"))?;
        let session = self.allocate_session(Session {
            program,
            execution,
            store,
            commit: active.cursor,
            state: Arc::new(loaded.state),
            pending: None,
            branch: active.selected_branch,
            branch_key,
            active_key,
            branch_revision,
            active_revision,
            catalog_revision: catalog.revision,
            coverage: catalog.coverage,
        })?;
        Ok(response::Body::SessionCreated(dto::SessionCreated {
            session,
            commit_id: active.cursor.as_bytes().to_vec(),
            snapshot,
        }))
    }

    fn capability_negotiation(
        &mut self,
        request: dto::CapabilityNegotiation,
    ) -> Result<response::Body, ProtocolDiagnostic> {
        let artifact = ProgramArtifactId::from_bytes(id32(&request.artifact_id, "Artifact ID")?);
        let program = self
            .programs
            .get(&artifact)
            .ok_or_else(|| ProtocolDiagnostic::missing("Program Artifact is not loaded"))?;
        let builtin = builtin_host_capabilities()
            .map_err(|error| ProtocolDiagnostic::capability(error.to_string()))?;
        let mut selected = Vec::new();
        for requested in request.host {
            let id = CapabilityId::new(&requested.id)
                .map_err(|error| ProtocolDiagnostic::capability(error.to_string()))?;
            let version = u16::try_from(requested.version)
                .ok()
                .and_then(CapabilityVersion::new)
                .ok_or_else(|| ProtocolDiagnostic::capability("invalid capability version"))?;
            if let Some(capability) = builtin.get(&id) {
                let mut capability = capability.clone();
                capability.version = version;
                selected.push(capability);
            }
        }
        let host = HostCapabilities::new(selected)
            .map_err(|error| ProtocolDiagnostic::capability(error.to_string()))?;
        let negotiated = negotiate_capabilities(&program.artifact().capabilities, &host)
            .map_err(|errors| ProtocolDiagnostic::capability(format!("{errors:?}")))?;
        let accepted = program
            .artifact()
            .capabilities
            .iter()
            .filter(|declaration| negotiated.contains(&declaration.id))
            .map(|declaration| dto::Capability {
                id: declaration.id.to_string(),
                version: u32::from(declaration.version.get()),
            })
            .collect();
        Ok(response::Body::Capabilities(dto::CapabilityResult {
            accepted,
        }))
    }

    fn ensure_session_capacity(&self) -> Result<(), ProtocolDiagnostic> {
        if self.sessions.len() as u64 >= self.limits.max_sessions {
            Err(ProtocolDiagnostic::limit("protocol sessions"))
        } else {
            Ok(())
        }
    }

    fn allocate_session(&mut self, session: Session) -> Result<u64, ProtocolDiagnostic> {
        let id = self.next_session;
        self.next_session = self
            .next_session
            .checked_add(1)
            .ok_or_else(|| ProtocolDiagnostic::limit("session handle space"))?;
        self.sessions.insert(id, session);
        Ok(id)
    }
}

fn run_slice(
    session_id: u64,
    session: &mut Session,
    runner: TransitionRunner,
    budget: SliceBudget,
    limits: ProtocolLimits,
) -> Result<response::Body, ProtocolDiagnostic> {
    match runner.run_slice(budget) {
        SliceOutcome::Yielded { runner, progress } => {
            session.pending = Some(runner);
            Ok(response::Body::SliceYielded(dto::SliceYielded {
                session: session_id,
                instruction_count: progress.instruction_count,
                call_count: progress.call_count,
                logical_alloc_units: progress.logical_alloc_units,
                microstep_count: progress.microstep_count,
                internal_event_count: progress.internal_event_count,
            }))
        }
        SliceOutcome::Completed(draft) => commit_draft(session_id, session, draft, limits),
        SliceOutcome::Faulted(error) => Err(ProtocolDiagnostic::runtime(error.to_string())),
    }
}

fn commit_draft(
    session_id: u64,
    session: &mut Session,
    draft: TransitionDraft,
    limits: ProtocolLimits,
) -> Result<response::Body, ProtocolDiagnostic> {
    let snapshot_bytes = export_snapshot(draft.next_state())
        .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
    let snapshot_object = checked_object(
        &snapshot_bytes,
        ObjectKind::Snapshot,
        0,
        limits.snapshot.decode.max_envelope_bytes,
    )?;
    let snapshot = SnapshotId::from_bytes(*snapshot_object.id().as_bytes());
    let receipt = TransitionReceiptV1::from_draft(
        session.execution,
        session.program.artifact_id(),
        session.commit,
        snapshot,
        &draft,
    );
    let receipt_object = receipt.to_object();
    if receipt_object.schema() != STORED_RECEIPT_SCHEMA_V1 {
        return Err(ProtocolDiagnostic::runtime("Receipt schema mismatch"));
    }
    let receipt_id = ReceiptId::from_bytes(*receipt_object.id().as_bytes());
    let commit_object = CommitV1 {
        parent: Some(session.commit),
        execution: session.execution,
        program: session.program.artifact_id(),
        snapshot,
        cause: CommitCauseV1::RuntimeTransition(receipt_id),
        ledger_fence: session
            .store
            .current_ledger_fence(session.execution)
            .map_err(store_diagnostic)?
            .get(),
        turn: draft.next_state().turn,
    }
    .to_object();
    let commit = CommitId::from_bytes(*commit_object.id().as_bytes());
    let catalog = session
        .store
        .read_catalog_head(&CatalogRefKey::new(session.execution))
        .map_err(store_diagnostic)?
        .ok_or_else(|| ProtocolDiagnostic::missing("Catalog head is missing"))?;
    if catalog.revision != session.catalog_revision || catalog.coverage != session.coverage {
        return Err(ProtocolDiagnostic::conflict(
            "Timeline Catalog changed outside this protocol session",
        ));
    }
    let catalog_event = TimelineCatalogEventV1 {
        execution: session.execution,
        previous: Some(catalog.event),
        operation: TimelineOperationId::from_bytes(*draft.input_id().as_bytes()),
        kind: TimelineCatalogEventKind::BranchAdvanced {
            branch: session.branch,
            previous_head: session.commit,
            next_head: commit,
        },
    }
    .to_object()
    .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
    let catalog_event_id =
        narrata_core::TimelineCatalogEventId::from_bytes(*catalog_event.id().as_bytes());
    let outcome = session
        .store
        .commit(CommitTransaction {
            objects: vec![
                snapshot_object,
                receipt_object,
                commit_object,
                catalog_event,
            ],
            refs: vec![
                RefMutation {
                    key: session.branch_key.clone(),
                    expected: Some(session.branch_revision),
                    next: Some(commit),
                },
                RefMutation {
                    key: session.active_key.clone(),
                    expected: Some(session.active_revision),
                    next: Some(commit),
                },
            ],
            catalogs: vec![CatalogMutation {
                key: CatalogRefKey::new(session.execution),
                expected: Some(session.catalog_revision),
                next: Some(catalog_event_id),
                coverage: session.coverage,
            }],
            inputs: vec![InputRecord {
                execution: session.execution,
                input: draft.input_id(),
                parent: session.commit,
                payload: draft.input_payload_digest(),
                commit,
            }],
            ..CommitTransaction::default()
        })
        .map_err(store_diagnostic)?;
    session.branch_revision = outcome
        .refs
        .get(&session.branch_key)
        .and_then(|value| *value)
        .map(|value| value.revision)
        .ok_or_else(|| ProtocolDiagnostic::missing("Branch Ref outcome is missing"))?;
    session.active_revision = outcome
        .refs
        .get(&session.active_key)
        .and_then(|value| *value)
        .map(|value| value.revision)
        .ok_or_else(|| ProtocolDiagnostic::missing("Active Ref outcome is missing"))?;
    session.catalog_revision = outcome
        .catalogs
        .get(&CatalogRefKey::new(session.execution))
        .and_then(|value| *value)
        .map(|value| value.revision)
        .ok_or_else(|| ProtocolDiagnostic::missing("Catalog outcome is missing"))?;
    let result = result_dto(draft.result());
    let digest = draft.next_state_digest();
    session.commit = commit;
    session.state = Arc::new(draft.into_next_state());
    Ok(response::Body::Committed(dto::CommittedRunResult {
        session: session_id,
        commit_id: commit.as_bytes().to_vec(),
        receipt_id: receipt_id.as_bytes().to_vec(),
        snapshot: snapshot_bytes,
        state_digest: digest.as_bytes().to_vec(),
        reused: false,
        result: Some(result),
    }))
}

fn reuse_committed(
    session_id: u64,
    session: &mut Session,
    commit: CommitId,
) -> Result<response::Body, ProtocolDiagnostic> {
    let loaded = load_commit(&session.store, commit, &session.program)
        .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
    let receipt = match loaded.commit.cause {
        CommitCauseV1::RuntimeTransition(receipt) => receipt,
        CommitCauseV1::Genesis | CommitCauseV1::Migration(_) => {
            return Err(ProtocolDiagnostic::runtime(
                "Input index points to a non-transition Commit",
            ));
        }
    };
    let snapshot = session
        .store
        .get_object(ObjectId::from_bytes(*loaded.commit.snapshot.as_bytes()))
        .map_err(store_diagnostic)?
        .ok_or_else(|| ProtocolDiagnostic::missing("Snapshot object is missing"))?
        .bytes()
        .to_vec();
    let result = result_from_state(&loaded.state)?;
    let digest = state_digest(&loaded.state);
    session.commit = commit;
    session.state = Arc::new(loaded.state);
    Ok(response::Body::Committed(dto::CommittedRunResult {
        session: session_id,
        commit_id: commit.as_bytes().to_vec(),
        receipt_id: receipt.as_bytes().to_vec(),
        snapshot,
        state_digest: digest.as_bytes().to_vec(),
        reused: true,
        result: Some(result_dto(&result)),
    }))
}

fn parse_input(
    input: dto::RuntimeInput,
    state: &RuntimeStateV0,
    program: &CheckedProgram,
    decode_limits: &DecodeLimits,
) -> Result<CheckedRuntimeInput, ProtocolDiagnostic> {
    let request = InputId::from_bytes(id16(&input.request_id, "Input ID")?);
    match input.kind {
        Some(runtime_input::Kind::Start(_)) => Ok(CheckedRuntimeInput::start(request)),
        Some(runtime_input::Kind::Advance(value)) => Ok(CheckedRuntimeInput::advance(
            request,
            InteractionId::from_bytes(id32(&value.interaction_id, "Interaction ID")?),
        )),
        Some(runtime_input::Kind::Select(value)) => Ok(CheckedRuntimeInput::select(
            request,
            InteractionId::from_bytes(id32(&value.interaction_id, "Interaction ID")?),
            ChoiceId::from_bytes(id16(&value.choice_id, "Choice ID")?),
        )),
        Some(runtime_input::Kind::Event(value)) => Ok(CheckedRuntimeInput::event(
            request,
            EventTypeId::from_bytes(id16(&value.event_type_id, "Event type ID")?),
        )),
        Some(runtime_input::Kind::EffectResponse(value)) => {
            let pending = state
                .pending_effect()
                .ok_or_else(|| ProtocolDiagnostic::invalid("Session is not awaiting an Effect"))?;
            let capability = CapabilityId::new(&value.capability)
                .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
            let version = u16::try_from(value.capability_version)
                .ok()
                .and_then(CapabilityVersion::new)
                .ok_or_else(|| ProtocolDiagnostic::invalid("invalid capability version"))?;
            let payload = decode_canonical_value(&value.canonical_value, decode_limits)
                .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
            CheckedRuntimeInput::effect_response(
                request,
                pending,
                program,
                EffectResponseV0 {
                    effect: EffectId::from_bytes(id32(&value.effect_id, "Effect ID")?),
                    request_digest: EffectRequestDigest::from_bytes(id32(
                        &value.request_digest,
                        "Effect request digest",
                    )?),
                    capability,
                    capability_version: version,
                    payload,
                },
            )
            .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))
        }
        None => Err(ProtocolDiagnostic::invalid("Runtime input kind is missing")),
    }
}

fn result_from_state(state: &RuntimeStateV0) -> Result<DraftResult, ProtocolDiagnostic> {
    match &state.status {
        RuntimeStatusV0::Awaiting { pending, .. } => match pending {
            PendingInteractionV0::Say {
                interaction_id,
                speaker,
                text,
                ..
            } => Ok(DraftResult::AwaitSay(SayView {
                interaction_id: *interaction_id,
                speaker: speaker.clone(),
                text: text.clone(),
            })),
            PendingInteractionV0::Choice {
                interaction_id,
                prompt,
                offered,
                ..
            } => Ok(DraftResult::AwaitChoice(ChoiceView {
                interaction_id: *interaction_id,
                prompt: prompt.clone(),
                choices: offered
                    .iter()
                    .map(|choice| ChoiceViewItem {
                        id: choice.id,
                        label: choice.label.clone(),
                    })
                    .collect(),
            })),
        },
        RuntimeStatusV0::AwaitingEffect { pending, .. }
        | RuntimeStatusV0::AwaitingStatechartEffect { pending } => {
            Ok(DraftResult::AwaitEffect(pending.request.clone()))
        }
        RuntimeStatusV0::Finished { result, .. } => Ok(DraftResult::Finished(result.clone())),
        RuntimeStatusV0::StatechartStable => state
            .statechart
            .as_ref()
            .map(|chart| DraftResult::StatechartStable(chart.into()))
            .ok_or_else(|| ProtocolDiagnostic::runtime("stable Statechart state is missing")),
        RuntimeStatusV0::StatechartFinished => Ok(DraftResult::Finished(Value::Null)),
        RuntimeStatusV0::Ready { .. } => Err(ProtocolDiagnostic::runtime(
            "transition ended outside a safe point",
        )),
    }
}

fn result_dto(result: &DraftResult) -> dto::RunResult {
    match result {
        DraftResult::AwaitSay(value) => dto::RunResult {
            kind: dto::RunResultKind::Say as i32,
            interaction_or_effect_id: value.interaction_id.as_bytes().to_vec(),
            text: value.text.to_string(),
            choices: Vec::new(),
            canonical_value: Vec::new(),
            active_states: Vec::new(),
        },
        DraftResult::AwaitChoice(value) => dto::RunResult {
            kind: dto::RunResultKind::Choice as i32,
            interaction_or_effect_id: value.interaction_id.as_bytes().to_vec(),
            text: value.prompt.as_deref().unwrap_or_default().to_owned(),
            choices: value
                .choices
                .iter()
                .map(|choice| dto::Choice {
                    id: choice.id.as_bytes().to_vec(),
                    label: choice.label.to_string(),
                })
                .collect(),
            canonical_value: Vec::new(),
            active_states: Vec::new(),
        },
        DraftResult::AwaitEffect(value) => dto::RunResult {
            kind: dto::RunResultKind::Effect as i32,
            interaction_or_effect_id: value.id.as_bytes().to_vec(),
            text: value.capability.to_string(),
            choices: Vec::new(),
            canonical_value: encode_canonical_value(&value.payload),
            active_states: Vec::new(),
        },
        DraftResult::Finished(value) => dto::RunResult {
            kind: dto::RunResultKind::Finished as i32,
            interaction_or_effect_id: Vec::new(),
            text: String::new(),
            choices: Vec::new(),
            canonical_value: encode_canonical_value(value),
            active_states: Vec::new(),
        },
        DraftResult::StatechartStable(value) => dto::RunResult {
            kind: dto::RunResultKind::StatechartStable as i32,
            interaction_or_effect_id: Vec::new(),
            text: String::new(),
            choices: Vec::new(),
            canonical_value: Vec::new(),
            active_states: value
                .active
                .iter()
                .map(|state| state.as_bytes().to_vec())
                .collect(),
        },
    }
}

fn checked_object(
    bytes: &[u8],
    kind: ObjectKind,
    schema: u16,
    max_bytes: u64,
) -> Result<CheckedObject, ProtocolDiagnostic> {
    CheckedObject::from_bytes(bytes, kind, schema, max_bytes)
        .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))
}

fn protocol_active_key() -> Result<RefKey, ProtocolDiagnostic> {
    let name =
        RefName::new("protocol").map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
    RefKey::active(name).map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))
}

fn outcome_ref_revision(
    outcome: &CommitOutcome,
    key: &RefKey,
) -> Result<RefRevision, ProtocolDiagnostic> {
    outcome
        .refs
        .get(key)
        .and_then(|value| *value)
        .map(|value| value.revision)
        .ok_or_else(|| {
            ProtocolDiagnostic::missing(format!("Ref {} outcome is missing", key.storage_key()))
        })
}

fn outcome_catalog_revision(
    outcome: &CommitOutcome,
    execution: ExecutionId,
) -> Result<RefRevision, ProtocolDiagnostic> {
    outcome
        .catalogs
        .get(&CatalogRefKey::new(execution))
        .and_then(|value| *value)
        .map(|value| value.revision)
        .ok_or_else(|| ProtocolDiagnostic::missing("Catalog outcome is missing"))
}

fn required_ref_revision(
    store: &MemoryStore,
    key: &RefKey,
) -> Result<RefRevision, ProtocolDiagnostic> {
    store
        .read_ref(key)
        .map_err(store_diagnostic)?
        .map(|value| value.revision)
        .ok_or_else(|| ProtocolDiagnostic::missing(format!("Ref {} is missing", key.storage_key())))
}

fn recording_started(
    execution: ExecutionId,
    branch: BranchId,
    baseline: CommitId,
) -> Result<CheckedObject, ProtocolDiagnostic> {
    TimelineCatalogEventV1 {
        execution,
        previous: None,
        operation: deterministic_operation(b"protocol-recording-started", baseline.as_bytes()),
        kind: TimelineCatalogEventKind::RecordingStarted {
            baseline,
            initial_refs: vec![ArchivedRefSnapshot::Branch(ArchivedBranchRef {
                branch,
                head: baseline,
            })],
        },
    }
    .to_object()
    .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))
}

fn deterministic_operation(domain: &[u8], source: &[u8]) -> TimelineOperationId {
    let mut hasher = Sha256::new();
    hasher.update(b"NARRATA-PROTOCOL-TIMELINE-OP\0");
    hasher.update(domain);
    hasher.update(source);
    let digest: [u8; 32] = hasher.finalize().into();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    TimelineOperationId::from_bytes(bytes)
}

fn bounded_slice(requested: u64, maximum: u64) -> Result<SliceBudget, ProtocolDiagnostic> {
    let work = if requested == 0 {
        maximum
    } else {
        requested.min(maximum)
    };
    SliceBudget::new(work).map_err(|error| ProtocolDiagnostic::limit(error.to_string()))
}

fn id16(bytes: &[u8], label: &'static str) -> Result<[u8; 16], ProtocolDiagnostic> {
    <[u8; 16]>::try_from(bytes)
        .map_err(|_| ProtocolDiagnostic::invalid(format!("{label} must contain 16 bytes")))
}

fn id32(bytes: &[u8], label: &'static str) -> Result<[u8; 32], ProtocolDiagnostic> {
    <[u8; 32]>::try_from(bytes)
        .map_err(|_| ProtocolDiagnostic::invalid(format!("{label} must contain 32 bytes")))
}

fn store_diagnostic(error: narrata_store::StoreError) -> ProtocolDiagnostic {
    ProtocolDiagnostic {
        code: "NAR-P0008",
        class: "store",
        message: error.to_string(),
        retryable: matches!(
            error,
            narrata_store::StoreError::Busy | narrata_store::StoreError::Full
        ),
    }
}

#[derive(Clone, Debug)]
struct ProtocolDiagnostic {
    code: &'static str,
    class: &'static str,
    message: String,
    retryable: bool,
}

impl ProtocolDiagnostic {
    fn invalid(message: impl Into<String>) -> Self {
        Self::new("NAR-P0001", "invalid-request", message)
    }

    fn incompatible(message: impl Into<String>) -> Self {
        Self::new("NAR-P0002", "incompatible", message)
    }

    fn missing(message: impl Into<String>) -> Self {
        Self::new("NAR-P0003", "missing", message)
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self::new("NAR-P0004", "conflict", message)
    }

    fn limit(message: impl Into<String>) -> Self {
        Self::new("NAR-P0005", "limit", message)
    }

    fn runtime(message: impl Into<String>) -> Self {
        Self::new("NAR-P0006", "runtime", message)
    }

    fn capability(message: impl Into<String>) -> Self {
        Self::new("NAR-P0007", "capability", message)
    }

    fn new(code: &'static str, class: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            class,
            message: message.into(),
            retryable: false,
        }
    }
}

fn diagnostic_response(request_id: u64, error: ProtocolDiagnostic) -> dto::Response {
    dto::Response {
        protocol_version: u32::from(PROTOCOL_V1.get()),
        request_id,
        body: Some(response::Body::Diagnostic(dto::Diagnostic {
            code: error.code.to_owned(),
            class: error.class.to_owned(),
            message: error.message,
            retryable: error.retryable,
        })),
    }
}

fn _ids_are_transport_only(_: (StateDigest, ReceiptId, EffectRequestDigest)) {}
