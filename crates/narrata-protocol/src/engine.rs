use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use narrata_core::{
    CapabilityId, CapabilityVersion, CheckedProgram, ChoiceId, EffectId, EffectRequestDigest,
    EventTypeId, ExecutionId, InputId, InteractionId, ObjectId, ProgramArtifactId,
    builtin_host_capabilities,
    codec::{ObjectKind, decode_canonical_value, encode_canonical_value},
    effect::{EffectResponseV0, HostCapabilities, negotiate_capabilities},
    limits::{DecodeLimits, MacrostepLimits, ProgramLoadLimits, SnapshotLoadLimits},
    program::{ContentRef, Segment, load_program},
    runtime::{
        CheckedRuntimeInput, ContentView, DraftResult, RuntimeStateV0, SliceBudget, SliceOutcome,
        TransitionRunner, begin_transition_with_parent_commit, new_execution,
    },
    snapshot::{export_snapshot, state_digest},
    version::{PROGRAM_FORMAT_V0, PROTOCOL_V2},
};
use narrata_storage::{MemoryBackend, StorageBackend, StorageError};
use narrata_store::{
    BranchId, BundleLimits, CheckpointBundle, CommitTransaction, CommitV1, CommittedRunResult,
    CoordinatorError, InitialRecordingMode, RefKey, RefMutation, RefName, SaveStore,
    SessionCoordinator, Store, TimelineArchiveBundle, TimelineImportMapping, TimelineOperationId,
    timeline_branch,
};
use prost::Message;
use thiserror::Error;

use crate::dto::{self, request, response, runtime_input};

/// Version 2 carries content references instead of reader text (ADR 0018).
pub const PROTOCOL_ABI_VERSION: u32 = 2;
/// The `protocol_version` every request must carry and every response carries.
pub const PROTOCOL_VERSION: u32 = PROTOCOL_V2.get() as u32;

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

struct Session<B> {
    program: Arc<CheckedProgram>,
    coordinator: SessionCoordinator<Store<B>>,
    pending: Option<TransitionRunner>,
}

pub struct ProtocolEngine<B = MemoryBackend> {
    limits: ProtocolLimits,
    programs: BTreeMap<ProgramArtifactId, Arc<CheckedProgram>>,
    sessions: BTreeMap<u64, Session<B>>,
    next_session: u64,
    backend_factory: Box<dyn FnMut(ExecutionId) -> Result<B, StorageError> + Send>,
}

impl Default for ProtocolEngine {
    fn default() -> Self {
        Self::new(ProtocolLimits::default())
    }
}

impl ProtocolEngine {
    pub fn new(limits: ProtocolLimits) -> Self {
        Self::with_backend_factory(limits, |_| Ok(MemoryBackend::default()))
    }
}

impl<B: StorageBackend> ProtocolEngine<B> {
    /// Opens a backend for each created, imported or reopened session. The host chooses
    /// the storage namespace from the Execution ID. Imports need a fresh namespace;
    /// `reopen_session` opens existing refs. Native and Wasm bindings use memory by default.
    pub fn with_backend_factory(
        limits: ProtocolLimits,
        factory: impl FnMut(ExecutionId) -> Result<B, StorageError> + Send + 'static,
    ) -> Self {
        Self {
            limits,
            programs: BTreeMap::new(),
            sessions: BTreeMap::new(),
            next_session: 1,
            backend_factory: Box::new(factory),
        }
    }

    /// Reopens a persisted session after its Program has been loaded. This Rust-only
    /// entry point keeps storage handles and callbacks outside the protocol ABI.
    pub fn reopen_session(
        &mut self,
        artifact: ProgramArtifactId,
        execution: ExecutionId,
        selected_branch: BranchId,
    ) -> Result<dto::SessionCreated, dto::Diagnostic> {
        self.open_session(artifact, execution, selected_branch)
            .map_err(ProtocolDiagnostic::into_dto)
    }

    fn open_session(
        &mut self,
        artifact: ProgramArtifactId,
        execution: ExecutionId,
        selected_branch: BranchId,
    ) -> Result<dto::SessionCreated, ProtocolDiagnostic> {
        self.ensure_session_capacity()?;
        let program = self.loaded_program(artifact)?;
        let host = builtin_host_capabilities()
            .map_err(|error| ProtocolDiagnostic::capability(error.to_string()))?;
        let store = self.open_store(execution)?;
        let coordinator = SessionCoordinator::open_with_capabilities(
            store,
            program.clone(),
            execution,
            protocol_session_name()?,
            selected_branch,
            &host,
        )
        .map_err(coordinator_diagnostic)?;
        self.insert_coordinator(program, coordinator)
    }

    fn open_store(&mut self, execution: ExecutionId) -> Result<Store<B>, ProtocolDiagnostic> {
        let backend =
            (self.backend_factory)(execution).map_err(|error| store_diagnostic(error.into()))?;
        Store::open(backend).map_err(store_diagnostic)
    }

    fn loaded_program(
        &self,
        artifact: ProgramArtifactId,
    ) -> Result<Arc<CheckedProgram>, ProtocolDiagnostic> {
        self.programs
            .get(&artifact)
            .cloned()
            .ok_or_else(|| ProtocolDiagnostic::missing("Program Artifact is not loaded"))
    }

    fn insert_coordinator(
        &mut self,
        program: Arc<CheckedProgram>,
        coordinator: SessionCoordinator<Store<B>>,
    ) -> Result<dto::SessionCreated, ProtocolDiagnostic> {
        let commit = coordinator.timeline().cursor;
        let execution = coordinator.state().execution_id;
        let snapshot = protocol_snapshot(coordinator.state(), self.limits)?;
        let session = self.allocate_session(Session {
            program,
            coordinator,
            pending: None,
        })?;
        Ok(dto::SessionCreated {
            session,
            commit_id: commit.as_bytes().to_vec(),
            snapshot,
            execution_id: execution.as_bytes().to_vec(),
        })
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
        if request.protocol_version != u32::from(PROTOCOL_V2.get()) {
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
                protocol_version: u32::from(PROTOCOL_V2.get()),
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
        // Hosts never receive text through the protocol; format 0 keeps it in the Program.
        if program.format_version() == PROGRAM_FORMAT_V0 {
            return Err(ProtocolDiagnostic::incompatible(
                "Program format 0 carries reader text; upgrade it to format 1 (ADR 0018)",
            ));
        }
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
        let program = self.loaded_program(artifact)?;
        // Enforce the boundary's Snapshot limit before a persistent backend can publish
        // Genesis. The coordinator owns Genesis construction and its atomic transaction.
        let initial_state = new_execution(&program, execution)
            .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        protocol_snapshot(&initial_state, self.limits)?;
        let host = builtin_host_capabilities()
            .map_err(|error| ProtocolDiagnostic::capability(error.to_string()))?;
        let store = self.open_store(execution)?;
        let coordinator = SessionCoordinator::create_with_capabilities(
            store,
            program.clone(),
            execution,
            protocol_session_name()?,
            BranchId::from_bytes(*execution.as_bytes()),
            InitialRecordingMode::Complete,
            0,
            &host,
        )
        .map_err(coordinator_diagnostic)?;
        self.insert_coordinator(program, coordinator)
            .map(response::Body::SessionCreated)
    }

    fn session_load(
        &mut self,
        artifact_bytes: Vec<u8>,
        bundle_bytes: Vec<u8>,
    ) -> Result<response::Body, ProtocolDiagnostic> {
        self.ensure_session_capacity()?;
        let artifact = ProgramArtifactId::from_bytes(id32(&artifact_bytes, "Artifact ID")?);
        let program = self.loaded_program(artifact)?;
        let bundle = CheckpointBundle::from_bytes(&bundle_bytes, self.limits.bundle)
            .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        let root = bundle.manifest.root;
        let commit_object = bundle
            .objects
            .iter()
            .find(|object| object.id() == ObjectId::from_bytes(*root.as_bytes()))
            .ok_or_else(|| ProtocolDiagnostic::missing("root Commit object is missing"))?;
        let commit = CommitV1::decode(commit_object.payload())
            .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        if commit.program != artifact {
            return Err(ProtocolDiagnostic::incompatible(
                "Checkpoint belongs to another Program",
            ));
        }
        let execution = commit.execution;
        let mut store = self.open_store(execution)?;
        let target = RefKey::active(protocol_session_name()?)
            .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        bundle
            .import(&mut store, target, None, 0)
            .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        let branch = BranchId::from_bytes(*execution.as_bytes());
        // Checkpoints omit timeline refs. Seed the selected branch, then let the
        // coordinator establish complete recording at the imported baseline.
        store
            .commit(CommitTransaction {
                refs: vec![RefMutation {
                    key: timeline_branch(execution, branch),
                    expected: None,
                    next: Some(root),
                }],
                ..CommitTransaction::default()
            })
            .map_err(store_diagnostic)?;
        let host = builtin_host_capabilities()
            .map_err(|error| ProtocolDiagnostic::capability(error.to_string()))?;
        let mut coordinator = SessionCoordinator::open_with_capabilities(
            store,
            program.clone(),
            execution,
            protocol_session_name()?,
            branch,
            &host,
        )
        .map_err(coordinator_diagnostic)?;
        coordinator
            .enable_complete_recording(TimelineOperationId::from_bytes(*execution.as_bytes()), 0)
            .map_err(coordinator_diagnostic)?;
        self.insert_coordinator(program, coordinator)
            .map(response::Body::SessionCreated)
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
        let budget = bounded_slice(request.slice_work, limits.max_slice_work)?;
        let input = parse_input(
            request
                .input
                .ok_or_else(|| ProtocolDiagnostic::invalid("Runtime input is missing"))?,
            session.coordinator.state(),
            &session.program,
            &limits.snapshot.decode,
        )?;
        if let Some(reused) = session
            .coordinator
            .reuse_input(&input)
            .map_err(coordinator_diagnostic)?
        {
            return committed_response(request.session, reused, limits);
        }
        let runner = begin_transition_with_parent_commit(
            session.program.clone(),
            session.coordinator.state().clone(),
            session.coordinator.timeline().cursor,
            input,
            limits.macrostep,
        )
        .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        run_slice(request.session, session, runner, budget, limits)
    }

    fn continue_slice(
        &mut self,
        request: dto::ContinueSlice,
    ) -> Result<response::Body, ProtocolDiagnostic> {
        let limits = self.limits;
        let budget = bounded_slice(request.slice_work, limits.max_slice_work)?;
        let session = self
            .sessions
            .get_mut(&request.session)
            .ok_or_else(|| ProtocolDiagnostic::invalid("invalid session handle"))?;
        let runner = session
            .pending
            .take()
            .ok_or_else(|| ProtocolDiagnostic::invalid("no transition slice is pending"))?;
        run_slice(request.session, session, runner, budget, limits)
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
        let bytes = CheckpointBundle::export(
            session.coordinator.store(),
            session.coordinator.timeline().cursor,
            &BTreeSet::new(),
        )
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
            session.coordinator.store(),
            session.coordinator.state().execution_id,
            Some(session.coordinator.timeline()),
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
        let session_name = protocol_session_name()?;
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
            session_name: Some(session_name.clone()),
        };
        let mut store = self.open_store(execution)?;
        bundle
            .import(&mut store, mapping, 0)
            .map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
        let host = builtin_host_capabilities()
            .map_err(|error| ProtocolDiagnostic::capability(error.to_string()))?;
        let coordinator = SessionCoordinator::open_with_capabilities(
            store,
            program.clone(),
            execution,
            session_name,
            active.selected_branch,
            &host,
        )
        .map_err(coordinator_diagnostic)?;
        self.insert_coordinator(program, coordinator)
            .map(response::Body::SessionCreated)
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

    fn allocate_session(&mut self, session: Session<B>) -> Result<u64, ProtocolDiagnostic> {
        let id = self.next_session;
        self.next_session = self
            .next_session
            .checked_add(1)
            .ok_or_else(|| ProtocolDiagnostic::limit("session handle space"))?;
        self.sessions.insert(id, session);
        Ok(id)
    }
}

fn run_slice<B: StorageBackend>(
    session_id: u64,
    session: &mut Session<B>,
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
        SliceOutcome::Completed(draft) => {
            // Reject an oversized Snapshot before publishing any refs.
            protocol_snapshot(draft.next_state(), limits)?;
            let parent = session.coordinator.timeline().cursor;
            let committed = session
                .coordinator
                .commit_draft(parent, draft, 0)
                .map_err(coordinator_diagnostic)?;
            committed_response(session_id, committed, limits)
        }
        SliceOutcome::Faulted(error) => Err(ProtocolDiagnostic::runtime(error.to_string())),
    }
}

fn committed_response(
    session: u64,
    committed: CommittedRunResult,
    limits: ProtocolLimits,
) -> Result<response::Body, ProtocolDiagnostic> {
    Ok(response::Body::Committed(dto::CommittedRunResult {
        session,
        commit_id: committed.commit.as_bytes().to_vec(),
        receipt_id: committed.receipt.as_bytes().to_vec(),
        snapshot: protocol_snapshot(&committed.state, limits)?,
        state_digest: state_digest(&committed.state).as_bytes().to_vec(),
        reused: committed.reused,
        result: Some(result_dto(&committed.result)?),
    }))
}

fn protocol_snapshot(
    state: &RuntimeStateV0,
    limits: ProtocolLimits,
) -> Result<Vec<u8>, ProtocolDiagnostic> {
    let snapshot =
        export_snapshot(state).map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))?;
    if snapshot.len() as u64 > limits.snapshot.decode.max_envelope_bytes {
        return Err(ProtocolDiagnostic::limit("Snapshot bytes"));
    }
    Ok(snapshot)
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

fn result_dto(result: &DraftResult) -> Result<dto::RunResult, ProtocolDiagnostic> {
    let empty = |kind: dto::RunResultKind| dto::RunResult {
        kind: kind as i32,
        interaction_or_effect_id: Vec::new(),
        choices: Vec::new(),
        canonical_value: Vec::new(),
        active_states: Vec::new(),
        speaker: None,
        body: None,
        prompt: None,
        capability: String::new(),
        occurrence: 0,
    };
    Ok(match result {
        DraftResult::AwaitSay(value) => dto::RunResult {
            interaction_or_effect_id: value.interaction_id.as_bytes().to_vec(),
            speaker: value.speaker.as_ref().map(reference_dto).transpose()?,
            body: Some(segment_dto(&value.text)?),
            occurrence: value.occurrence,
            ..empty(dto::RunResultKind::Say)
        },
        DraftResult::AwaitChoice(value) => dto::RunResult {
            interaction_or_effect_id: value.interaction_id.as_bytes().to_vec(),
            prompt: value.prompt.as_ref().map(reference_dto).transpose()?,
            choices: value
                .choices
                .iter()
                .map(|choice| {
                    Ok(dto::Choice {
                        id: choice.id.as_bytes().to_vec(),
                        label: Some(reference_dto(&choice.label)?),
                    })
                })
                .collect::<Result<_, ProtocolDiagnostic>>()?,
            occurrence: value.occurrence,
            ..empty(dto::RunResultKind::Choice)
        },
        DraftResult::AwaitEffect(value) => dto::RunResult {
            interaction_or_effect_id: value.id.as_bytes().to_vec(),
            capability: value.capability.to_string(),
            canonical_value: encode_canonical_value(&value.payload),
            ..empty(dto::RunResultKind::Effect)
        },
        DraftResult::Finished(value) => dto::RunResult {
            canonical_value: encode_canonical_value(value),
            ..empty(dto::RunResultKind::Finished)
        },
        DraftResult::StatechartStable(value) => dto::RunResult {
            active_states: value
                .active
                .iter()
                .map(|state| state.as_bytes().to_vec())
                .collect(),
            ..empty(dto::RunResultKind::StatechartStable)
        },
    })
}

fn content_ref_dto(reference: &ContentRef) -> dto::ContentRef {
    dto::ContentRef {
        provider: reference.provider.to_string(),
        key: reference.key.to_string(),
    }
}

/// Format 1 validation makes speakers, prompts and labels references and bodies segments;
/// only a format 0 Program, which `ProgramLoad` refuses, yields text.
fn reference_dto(view: &ContentView) -> Result<dto::ContentRef, ProtocolDiagnostic> {
    match view {
        ContentView::Ref(reference) => Ok(content_ref_dto(reference)),
        ContentView::Segment(_) | ContentView::LegacyText(_) => Err(
            ProtocolDiagnostic::incompatible("result content is not a content reference"),
        ),
    }
}

fn segment_dto(view: &ContentView) -> Result<dto::Segment, ProtocolDiagnostic> {
    match view {
        ContentView::Segment(Segment { unit, first, last }) => Ok(dto::Segment {
            unit: Some(content_ref_dto(unit)),
            first: first.as_ref().map(ToString::to_string),
            last: last.as_ref().map(ToString::to_string),
        }),
        ContentView::Ref(_) | ContentView::LegacyText(_) => Err(ProtocolDiagnostic::incompatible(
            "result content is not a content segment",
        )),
    }
}

fn protocol_session_name() -> Result<RefName, ProtocolDiagnostic> {
    RefName::new("protocol").map_err(|error| ProtocolDiagnostic::invalid(error.to_string()))
}

fn coordinator_diagnostic(error: CoordinatorError) -> ProtocolDiagnostic {
    match error {
        CoordinatorError::Store(error) => match error {
            narrata_store::StoreError::InputConflict(_)
            | narrata_store::StoreError::RefConflict(_)
            | narrata_store::StoreError::CatalogConflict(_) => {
                ProtocolDiagnostic::conflict(error.to_string())
            }
            error => store_diagnostic(error),
        },
        CoordinatorError::CapabilityNegotiation(_) | CoordinatorError::CapabilityUnavailable(_) => {
            ProtocolDiagnostic::capability(error.to_string())
        }
        CoordinatorError::Runtime(_) => ProtocolDiagnostic::runtime(error.to_string()),
        error => ProtocolDiagnostic::invalid(error.to_string()),
    }
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
    fn into_dto(self) -> dto::Diagnostic {
        dto::Diagnostic {
            code: self.code.to_owned(),
            class: self.class.to_owned(),
            message: self.message,
            retryable: self.retryable,
        }
    }
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
        protocol_version: u32::from(PROTOCOL_V2.get()),
        request_id,
        body: Some(response::Body::Diagnostic(error.into_dto())),
    }
}
