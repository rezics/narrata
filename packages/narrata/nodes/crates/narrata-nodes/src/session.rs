//! Node sessions driven by the kernel history layer. JSON R2 exports are import-only.

use crate::{
    ArtifactId, ChoicePointId, ChunkPins, CommitId, Error, ExecutionId, ObjectId, OptionId,
    Program, ProposalRequest, Result,
    history::{NodeDomain, history_error},
    json::parse_json_limited,
    plan::NameTable,
    runtime::{Item, Machine, Step},
    state::{Commit, Input, State, decode_input, decode_state},
    view::{FrameView, HistoryView, PresentationItem, ProductView, SessionView, VariableView},
    wire,
};
use narrata_history::{
    AdvanceError, AuditError, BundleError, BundleLimits, DomainKinds, History, HistoryError,
    Object, RefKey, RefMutation, RefName, RefNamespace, RefScope, Transaction,
};
use narrata_storage::{MemoryBackend, StorageBackend};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Limits of the old JSON importer, not limits of a kernel session.
pub const MAX_COMMITS: usize = 512;
pub const MAX_RETAINED_STATE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_EXPORT_BYTES: usize = 64 * 1024 * 1024;
const MAX_INTERIM_EXPORT_BYTES: usize = 8 * 1024 * 1024;
pub const EXPORT_FORMAT_VERSION: u16 = 2;

#[derive(Debug)]
struct Record {
    id: CommitId,
    commit: Commit,
    state: Arc<State>,
}

/// Only the current decoded state and its live frame pins are held by the coordinator.
pub struct Session<B = MemoryBackend> {
    program: Arc<Program>,
    programs: BTreeMap<ArtifactId, Arc<Program>>,
    execution: ExecutionId,
    history: History<B, DomainKinds<NodeDomain>>,
    session: narrata_history::Session<NodeDomain>,
    current: Record,
    _pins: ChunkPins,
    prefetched: Vec<Arc<crate::plan::Graph>>,
}

/// Read-only compatibility with the temporary R2 JSON container.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionExport {
    pub format_version: u16,
    pub artifact_id: ArtifactId,
    pub execution: ExecutionId,
    pub cursor: CommitId,
    pub objects: Vec<String>,
}

fn kernel(id: CommitId) -> narrata_history::ObjectId {
    narrata_history::ObjectId::from_bytes(*id.as_bytes())
}
fn node(id: narrata_history::ObjectId) -> CommitId {
    CommitId::from_bytes(*id.as_bytes())
}
fn record(loaded: narrata_history::Loaded<State>) -> Record {
    let h = loaded.header;
    Record {
        id: node(loaded.commit),
        commit: Commit {
            artifact: ArtifactId::from_bytes(*h.artifact.as_bytes()),
            parent: h.parent.map(node),
            input: h.input.map(|id| ObjectId::from_bytes(*id.as_bytes())),
            state: ObjectId::from_bytes(*h.state.as_bytes()),
            depth: h.depth,
        },
        state: Arc::new(loaded.state),
    }
}
fn name(execution: ExecutionId) -> RefName {
    RefName::hex(execution.as_bytes())
}
fn bundle_error(error: BundleError<HistoryError>) -> Error {
    match error {
        BundleError::Store(error) => history_error(error),
        other => Error::new("save", "checkpoint", other.to_string()),
    }
}

impl Session {
    /// Resolves the save's checked root header, never the work's current publication.
    /// Descriptors are not needed to restore an already committed target state.
    pub fn restore_with_registry(
        registry: &crate::history::evolution::EvolutionRegistry,
        text: &str,
    ) -> Result<Self> {
        let artifact = if text.trim_start().starts_with('{') {
            parse_json_limited::<SessionExport>(text, MAX_INTERIM_EXPORT_BYTES)?.artifact_id
        } else {
            if text.len() > MAX_EXPORT_BYTES {
                return Err(Error::new(
                    "limit",
                    "checkpoint",
                    "checkpoint exceeds 64 MiB",
                ));
            }
            let bytes = hex::decode(text.trim())
                .map_err(|e| Error::new("decode", "checkpoint", e.to_string()))?;
            let (root, objects) = checkpoint_objects(&bytes)?;
            let object = objects.iter().find(|o| o.id() == root).ok_or_else(|| {
                Error::new(
                    "needs_object",
                    root.to_string(),
                    "checkpoint root is missing",
                )
            })?;
            Commit::decode(object.bytes())?.artifact
        };
        let mut session = Self::restore(registry.program(artifact)?.clone(), text)?;
        session.programs.extend(registry.programs.clone());
        Ok(session)
    }
    pub fn new(program: Arc<Program>, execution: ExecutionId) -> Result<Self> {
        Self::open(program, execution, MemoryBackend::new())
    }
    /// Restores a hexadecimal kernel checkpoint, or imports a legacy JSON session.
    /// Checkpoints carry the cursor and its ancestors; live-store sibling branches stay there.
    pub fn restore(program: Arc<Program>, text: &str) -> Result<Self> {
        if text.trim_start().starts_with('{') {
            let (execution, cursor, objects, commits) = legacy(&program, text)?;
            let mut history = History::in_memory(DomainKinds::<NodeDomain>::new());
            let refs = commits
                .into_iter()
                .map(|id| RefMutation {
                    key: RefKey::bookmark(name(execution), RefName::hex(id.as_bytes())),
                    expected: None,
                    next: Some(kernel(id)),
                })
                .chain(std::iter::once(RefMutation {
                    key: RefKey::active(name(execution))
                        .map_err(|e| Error::new("name", "session", e.to_string()))?,
                    expected: None,
                    next: Some(kernel(cursor)),
                }))
                .collect();
            history
                .write(
                    &Transaction {
                        objects,
                        refs,
                        ..Transaction::default()
                    },
                    |_| Ok(Vec::new()),
                )
                .map_err(history_error)?;
            return Self::open(program, execution, history.into_backend());
        }
        if text.len() > MAX_EXPORT_BYTES {
            return Err(Error::new("limit", "export", "checkpoint exceeds 64 MiB"));
        }
        let bytes = hex::decode(text.trim())
            .map_err(|e| Error::new("decode", "checkpoint", e.to_string()))?;
        let root = checkpoint_root(&bytes)?;
        let mut id = [0; 16];
        id.copy_from_slice(&root.as_bytes()[..16]);
        let execution = ExecutionId::from_bytes(id);
        let mut history = History::in_memory(DomainKinds::<NodeDomain>::new());
        history
            .import(
                &NodeDomain(program.clone()),
                &bytes,
                BundleLimits::default(),
                RefKey::active(name(execution))
                    .map_err(|e| Error::new("name", "session", e.to_string()))?,
                None,
                0,
            )
            .map_err(bundle_error)?;
        Self::open(program, execution, history.into_backend())
    }
}

impl<B: StorageBackend> Session<B> {
    /// Opens the exact artifact named by the active cursor. An absent cursor is an error;
    /// starting a new execution remains the explicit single-program `open` operation.
    pub fn open_with_registry(
        registry: &crate::history::evolution::EvolutionRegistry,
        execution: ExecutionId,
        backend: B,
    ) -> Result<Self> {
        let location = crate::history::locator::locate(&backend, execution)
            .map_err(history_error)?
            .ok_or_else(|| Error::new("save", "cursor", "missing active cursor"))?;
        let mut session = Self::resume(
            registry.program(location.artifact)?.clone(),
            execution,
            backend,
        )?;
        if session.cursor()? != location.cursor {
            return Err(Error::new(
                "stale_input",
                "cursor",
                "active cursor moved during artifact discovery",
            ));
        }
        session.programs.extend(registry.programs.clone());
        Ok(session)
    }
    /// Adds an exact historic program for page rendering and checkout, without upgrading.
    pub fn register_history_program(&mut self, program: Arc<Program>) -> Result<()> {
        program.verify_artifact()?;
        self.programs.insert(program.artifact_id(), program);
        Ok(())
    }
    fn header(&self, id: CommitId) -> Result<Commit> {
        let object = self
            .history
            .reader()
            .require(kernel(id), Some(wire::KIND_COMMIT))
            .map_err(history_error)?;
        Commit::decode(object.bytes())
    }
    fn program_at(&self, artifact: ArtifactId) -> Result<&Arc<Program>> {
        self.programs.get(&artifact).ok_or_else(|| {
            Error::new(
                "needs_artifact",
                artifact.to_string(),
                "exact historic program is unavailable",
            )
        })
    }
    fn reopen(&mut self, program: Arc<Program>) -> Result<()> {
        let (session, loaded) = narrata_history::Session::open(
            &self.history,
            NodeDomain(program.clone()),
            name(self.execution),
        )
        .map_err(history_error)?;
        let pins = program.pin_state(&loaded.state)?;
        self.programs.insert(program.artifact_id(), program.clone());
        self.program = program;
        self.session = session;
        self.current = record(loaded);
        self._pins = pins;
        self.prefetched.clear();
        Ok(())
    }
    /// Reopens this execution's cursor without replay; creates its root only when absent.
    pub fn open(program: Arc<Program>, execution: ExecutionId, backend: B) -> Result<Self> {
        Self::open_mode(program, execution, backend, true)
    }
    /// Reopens only: missing metadata or cursor is an error and never causes a write.
    pub fn resume(program: Arc<Program>, execution: ExecutionId, backend: B) -> Result<Self> {
        Self::open_mode(program, execution, backend, false)
    }
    fn open_mode(
        program: Arc<Program>,
        execution: ExecutionId,
        backend: B,
        create: bool,
    ) -> Result<Self> {
        let mut history = if create {
            History::open(backend, DomainKinds::<NodeDomain>::new())
        } else {
            History::open_existing(backend, DomainKinds::<NodeDomain>::new())
        }
        .map_err(history_error)?;
        let domain = NodeDomain(program.clone());
        let mut initial_pins = None;
        let (session, loaded) =
            match narrata_history::Session::open(&history, domain.clone(), name(execution)) {
                Err(HistoryError::MissingRef(_)) if create => {
                    let initial = Machine { program: &program }.initial()?;
                    initial_pins = Some(initial.pins);
                    narrata_history::Session::create(
                        &mut history,
                        domain,
                        name(execution),
                        &initial.state,
                        0,
                    )
                }
                other => other,
            }
            .map_err(history_error)?;
        let pins = match initial_pins {
            Some(pins) => pins,
            None => program.pin_state(&loaded.state)?,
        };
        Ok(Self {
            programs: [(program.artifact_id(), program.clone())].into(),
            program,
            execution,
            history,
            session,
            current: record(loaded),
            _pins: pins,
            prefetched: Vec::new(),
        })
    }
    pub fn program(&self) -> &Arc<Program> {
        &self.program
    }
    pub fn execution(&self) -> ExecutionId {
        self.execution
    }
    /// Hosts can run this between interactions; it never changes narrative state or refs.
    pub fn prefetch(&mut self) -> Result<()> {
        self.prefetched = Machine {
            program: &self.program,
        }
        .prefetch(&self.current.state)?;
        Ok(())
    }
    pub fn history(&self) -> &History<B, DomainKinds<NodeDomain>> {
        &self.history
    }
    pub fn into_backend(self) -> B {
        self.history.into_backend()
    }
    pub fn cursor(&self) -> Result<CommitId> {
        Ok(self.current.id)
    }
    pub fn state(&self) -> Result<&State> {
        Ok(&self.current.state)
    }
    /// Current interaction needs only the exact restored state, including shallow saves.
    pub fn interaction(&self, names: Option<&NameTable>) -> Result<crate::view::Interaction> {
        (Machine {
            program: &self.program,
        })
        .interaction(&self.current.state, names)
    }
    fn load(&self, id: CommitId) -> Result<Record> {
        let header = self.header(id)?;
        let program = self.program_at(header.artifact)?;
        self.history
            .load(&NodeDomain(program.clone()), kernel(id))
            .map(record)
            .map_err(history_error)
    }
    /// Kernel children pagination; indexes are hints checked by the kernel when loaded.
    pub fn children(
        &self,
        parent: CommitId,
        after: Option<CommitId>,
        limit: u32,
    ) -> Result<narrata_history::Page<narrata_history::ObjectId>> {
        self.history
            .children(kernel(parent), after.map(kernel).as_ref(), limit)
            .map_err(history_error)
    }
    /// Headers reachable from this session's cursor, branches and imported legacy bookmarks.
    pub fn commits(&self) -> Result<impl Iterator<Item = (CommitId, Commit)>> {
        let mut pending = vec![self.current.id];
        for namespace in [RefNamespace::Branch, RefNamespace::Bookmark] {
            let mut after = None;
            loop {
                let page = self
                    .history
                    .scan_refs(
                        &RefScope::Owner(namespace, name(self.execution)),
                        after.as_ref(),
                        256,
                    )
                    .map_err(history_error)?;
                pending.extend(page.items.iter().map(|(_, value)| node(value.commit)));
                if !page.more {
                    break;
                }
                after = page.items.last().map(|(key, _)| key.clone());
            }
        }
        let mut seen = BTreeMap::new();
        while let Some(id) = pending.pop() {
            if seen.contains_key(&id) {
                continue;
            }
            let r = self.load(id)?;
            pending.extend(r.commit.parent);
            seen.insert(id, r.commit);
        }
        let mut records: Vec<_> = seen.into_iter().collect();
        records.sort_by_key(|(id, c)| (c.depth, *id));
        Ok(records.into_iter())
    }
    fn advance(&mut self, expected: &CommitId, input: Input) -> Result<CommitId> {
        if *expected != self.current.id {
            return Err(Error::new(
                "stale_input",
                "expected_commit",
                "the displayed interaction is no longer current",
            ));
        }
        decode_input(
            &self.program,
            expected,
            &self.current.state,
            &input.envelope(),
        )?;
        // A reused transition can disappear under concurrent GC. Retry that specific race once.
        for attempt in 0..2 {
            let mut step_pins = None;
            let result = self.session.advance(
                &mut self.history,
                kernel(*expected),
                &input,
                |domain, parent, input| {
                    let step = Machine { program: &domain.0 }.apply(expected, parent, input)?;
                    step_pins = Some(step.pins);
                    Ok::<_, Error>(step.state)
                },
                0,
            );
            match result {
                Ok(advanced) => {
                    let pins = match step_pins {
                        Some(pins) => pins,
                        None => self.program.pin_state(&advanced.state)?,
                    };
                    self.current = Record {
                        id: node(advanced.commit),
                        commit: Commit {
                            artifact: self.program.artifact_id(),
                            parent: Some(*expected),
                            input: Some(input.id()),
                            state: advanced.state.id(),
                            depth: advanced.depth,
                        },
                        state: Arc::new(advanced.state),
                    };
                    self.prefetched.clear();
                    self._pins = pins;
                    return Ok(self.current.id);
                }
                Err(AdvanceError::History(HistoryError::MissingObject(_))) if attempt == 0 => {
                    continue;
                }
                Err(AdvanceError::History(e)) => return Err(history_error(e)),
                Err(AdvanceError::Step(e)) => return Err(e),
            }
        }
        Err(Error::new(
            "history",
            "advance",
            "transition disappeared twice",
        ))
    }
    pub fn choose(
        &mut self,
        expected: &CommitId,
        choice_point: ChoicePointId,
        options: Vec<OptionId>,
    ) -> Result<CommitId> {
        self.advance(
            expected,
            Input::Choose {
                choice_point,
                options,
            },
        )
    }
    pub fn propose(&mut self, expected: &CommitId, request: &ProposalRequest) -> Result<CommitId> {
        self.advance(expected, crate::proposal::materialize(expected, request)?)
    }
    pub fn checkout(&mut self, commit: &CommitId) -> Result<()> {
        let target = self.load(*commit)?;
        let program = self.program_at(target.commit.artifact)?.clone();
        let pins = program.pin_state(&target.state)?;
        if program.artifact_id() != self.program.artifact_id() {
            let key = RefKey::active(name(self.execution))
                .map_err(|e| Error::new("name", "session", e.to_string()))?;
            let cursor = self
                .history
                .read_ref(&key)
                .map_err(history_error)?
                .ok_or_else(|| Error::new("stale_input", "cursor", "missing active cursor"))?;
            if cursor.commit != kernel(self.current.id) {
                return Err(Error::new("stale_input", "cursor", "active cursor moved"));
            }
            self.history
                .write_atomic(
                    &Transaction {
                        refs: vec![RefMutation {
                            key,
                            expected: Some(cursor.revision),
                            next: Some(kernel(*commit)),
                        }],
                        ..Transaction::default()
                    },
                    |_| Ok(Vec::new()),
                )
                .map_err(history_error)?;
            self.reopen(program)?;
            return Ok(());
        }
        let loaded = self
            .session
            .checkout(
                &mut self.history,
                kernel(self.current.id),
                kernel(*commit),
                0,
            )
            .map_err(history_error)?;
        self.current = record(loaded);
        self.prefetched.clear();
        self._pins = pins;
        Ok(())
    }
    pub fn save(&mut self, slot: RefName) -> Result<()> {
        let key = RefKey::save(name(self.execution), slot);
        let expected = self
            .history
            .read_ref(&key)
            .map_err(history_error)?
            .map(|v| v.revision);
        self.session
            .save(&mut self.history, key, expected, 0)
            .map_err(history_error)?;
        Ok(())
    }
    pub fn load_save(&mut self, slot: RefName) -> Result<()> {
        let key = RefKey::save(name(self.execution), slot);
        let target = self
            .history
            .read_ref(&key)
            .map_err(history_error)?
            .ok_or_else(|| Error::new("save", "slot", "missing save slot"))?;
        if self.header(node(target.commit))?.artifact != self.program.artifact_id() {
            return self.checkout(&node(target.commit));
        }
        let pins = self
            .program
            .pin_state(&self.load(node(target.commit))?.state)?;
        let loaded = self
            .session
            .load_save(&mut self.history, &key, 0)
            .map_err(history_error)?;
        self.current = record(loaded);
        self.prefetched.clear();
        self._pins = pins;
        Ok(())
    }
    fn step(&self, id: CommitId) -> Result<Step> {
        let record = self.load(id)?;
        let program = self.program_at(record.commit.artifact)?;
        let machine = Machine { program };
        let step = match (record.commit.parent, record.commit.input) {
            (Some(parent), Some(input)) => {
                let object = self
                    .history
                    .reader()
                    .require(
                        narrata_history::ObjectId::from_bytes(*input.as_bytes()),
                        None,
                    )
                    .map_err(history_error)?;
                if object.kind() == narrata_history::migration::MIGRATION_INPUT_KIND {
                    let edge = narrata_history::migration::MigrationInput::from_object(&object)
                        .map_err(history_error)?;
                    let parent_header = self.header(parent)?;
                    if edge.from.as_bytes() != parent_header.artifact.as_bytes()
                        || edge.to.as_bytes() != record.commit.artifact.as_bytes()
                    {
                        return Err(Error::new(
                            "save",
                            "migration",
                            "migration artifact link differs",
                        ));
                    }
                    return crate::runtime::migration_page(program, &record.state);
                }
                let parent = self.load(parent)?;
                if parent.commit.artifact != record.commit.artifact {
                    return Err(Error::new(
                        "save",
                        "input",
                        "ordinary input changed artifact",
                    ));
                }
                let input_parent = self.input_parent(parent.id, &parent.state, object.bytes())?;
                let (input, _) =
                    decode_input(program, &input_parent, &parent.state, object.bytes())?;
                machine.apply(&input_parent, &parent.state, &input)?
            }
            _ => machine.initial()?,
        };
        if step.state != *record.state {
            return Err(Error::new(
                "unreachable_state",
                id.to_string(),
                "the recorded state is not what its parent and input produce",
            ));
        }
        Ok(step)
    }
    pub fn presentation(&self, commit: &CommitId) -> Result<Vec<PresentationItem>> {
        Ok(items(*commit, self.step(*commit)?.presentation))
    }
    pub fn page(&self) -> Result<Vec<PresentationItem>> {
        let mut parts = Vec::new();
        let mut id = self.current.id;
        loop {
            let step = self.step(id)?;
            let record = self.load(id)?;
            let entered = step.entered || record.state.finished.is_some();
            parts.push(items(id, step.presentation));
            match (record.commit.parent, entered) {
                (Some(parent), false) => id = parent,
                _ => break,
            }
        }
        Ok(parts.into_iter().rev().flatten().collect())
    }
    fn interim_id(&self, id: CommitId) -> Result<CommitId> {
        let mut path = Vec::new();
        let mut next = Some(id);
        while let Some(id) = next {
            let r = self.load(id)?;
            next = r.commit.parent;
            path.push(r.commit);
        }
        let mut parent = None;
        for mut commit in path.into_iter().rev() {
            commit.parent = parent;
            parent = Some(CommitId::from_bytes(
                *wire::id_of(wire::KIND_COMMIT, &wire::encode_interim_commit(&commit)).as_bytes(),
            ));
        }
        parent.ok_or_else(|| Error::new("save", "path", "missing root"))
    }
    fn input_parent(&self, id: CommitId, state: &State, bytes: &[u8]) -> Result<CommitId> {
        let header = self.header(id)?;
        let program = self.program_at(header.artifact)?;
        match decode_input(program, &id, state, bytes) {
            Ok(_) => Ok(id),
            Err(error) if error.code == "derived_id" => {
                let old = self.interim_id(id)?;
                decode_input(program, &old, state, bytes)?;
                Ok(old)
            }
            Err(error) => Err(error),
        }
    }
    pub fn verify_path(&self, commit: &CommitId) -> Result<()> {
        let mut root = self.load(*commit)?;
        while let Some(parent) = root.commit.parent {
            root = self.load(parent)?;
        }
        if (Machine {
            program: &self.program,
        })
        .initial()?
        .state
            != *root.state
        {
            return Err(Error::new(
                "unreachable_state",
                "root",
                "root differs from the initial state",
            ));
        }
        let mut parent = root.id;
        let mut depth = 0;
        self.history
            .verify_path(
                self.session.domain(),
                kernel(*commit),
                |domain, state, input| {
                    let input_parent = self.input_parent(parent, state, &input.envelope())?;
                    let next = Machine { program: &domain.0 }
                        .apply(&input_parent, state, input)?
                        .state;
                    depth += 1;
                    parent = Commit {
                        artifact: self.program.artifact_id(),
                        parent: Some(parent),
                        input: Some(input.id()),
                        state: next.id(),
                        depth,
                    }
                    .id();
                    Ok::<_, Error>(next)
                },
            )
            .map_err(|error| match error {
                AuditError::History(e) => history_error(e),
                AuditError::Step(e) => e,
                AuditError::Diverged { .. } => {
                    Error::new("unreachable_state", commit.to_string(), error.to_string())
                }
            })
    }
    /// Hexadecimal transport of the kernel's checkpoint bytes, with no node-owned container.
    pub fn export(&self) -> Result<String> {
        let bundle = self
            .history
            .export(kernel(self.current.id), &BTreeSet::new())
            .map_err(bundle_error)?;
        // Include the manifest envelope and per-object headers before allocating the
        // container. Hex transport doubles the container size.
        let total = bundle
            .manifest
            .objects
            .iter()
            .try_fold(0_u64, |total, object| total.checked_add(object.bytes))
            .unwrap_or(u64::MAX)
            .saturating_add(bundle.manifest.encode().len() as u64 + 80)
            .saturating_add(bundle.manifest.objects.len() as u64 * 44);
        if total > (MAX_EXPORT_BYTES / 2) as u64
            || bundle.manifest.objects.len() as u64 > BundleLimits::default().max_objects
        {
            return Err(Error::new(
                "limit",
                "export",
                "checkpoint exceeds 64 MiB of hex",
            ));
        }
        let bytes = bundle
            .to_bytes()
            .map_err(|e| Error::new("save", "checkpoint", e.to_string()))?;
        if bytes.len() > MAX_EXPORT_BYTES / 2 {
            return Err(Error::new(
                "limit",
                "export",
                "checkpoint exceeds 64 MiB of hex",
            ));
        }
        Ok(hex::encode(bytes))
    }
    /// Imports into this backend and moves this execution's cursor only after checked import.
    pub fn import(&mut self, text: &str) -> Result<()> {
        if text.trim_start().starts_with('{') {
            let (_, cursor, objects, commits) = legacy(&self.program, text)?;
            return self.import_objects(objects, commits, cursor);
        }
        if text.len() > MAX_EXPORT_BYTES {
            return Err(Error::new(
                "limit",
                "checkpoint",
                "checkpoint exceeds 64 MiB",
            ));
        }
        let bytes = hex::decode(text.trim())
            .map_err(|e| Error::new("decode", "checkpoint", e.to_string()))?;
        let (root, objects) = checkpoint_objects(&bytes)?;
        let header = Commit::decode(
            objects
                .iter()
                .find(|o| o.id() == root)
                .ok_or_else(|| {
                    Error::new(
                        "needs_object",
                        root.to_string(),
                        "checkpoint root is missing",
                    )
                })?
                .bytes(),
        )?;
        let program = self
            .programs
            .get(&header.artifact)
            .ok_or_else(|| {
                Error::new(
                    "incompatible_save",
                    "artifact_id",
                    "the export belongs to another artifact",
                )
            })?
            .clone();
        let key = RefKey::active(name(self.execution))
            .map_err(|e| Error::new("name", "session", e.to_string()))?;
        let expected = self
            .history
            .read_ref(&key)
            .map_err(history_error)?
            .map(|v| v.revision);
        self.history
            .import(
                &NodeDomain(program.clone()),
                &bytes,
                BundleLimits::default(),
                key,
                expected,
                0,
            )
            .map_err(bundle_error)?;
        self.reopen(program)
    }
    /// Transfers all retained native branches, for R1 migration.
    pub fn import_session(&mut self, source: &Session) -> Result<()> {
        if source.program.artifact_id() != self.program.artifact_id() {
            return Err(Error::new(
                "incompatible_save",
                "artifact_id",
                "session belongs to another artifact",
            ));
        }
        let commits: Vec<_> = source.commits()?.map(|(id, _)| id).collect();
        let mut objects = BTreeMap::new();
        for id in &commits {
            let r = source.load(*id)?;
            for object_id in [
                Some(kernel(*id)),
                Some(narrata_history::ObjectId::from_bytes(
                    *r.commit.state.as_bytes(),
                )),
                r.commit
                    .input
                    .map(|id| narrata_history::ObjectId::from_bytes(*id.as_bytes())),
            ]
            .into_iter()
            .flatten()
            {
                let object = source
                    .history
                    .reader()
                    .require(object_id, None)
                    .map_err(history_error)?;
                objects.insert(object_id, object);
            }
        }
        self.import_objects(objects.into_values().collect(), commits, source.current.id)?;
        self.programs.extend(source.programs.clone());
        Ok(())
    }
    fn import_objects(
        &mut self,
        objects: Vec<Object>,
        commits: Vec<CommitId>,
        cursor: CommitId,
    ) -> Result<()> {
        let key = RefKey::active(name(self.execution))
            .map_err(|e| Error::new("name", "session", e.to_string()))?;
        let expected = self
            .history
            .read_ref(&key)
            .map_err(history_error)?
            .map(|v| v.revision);
        let mut refs = vec![RefMutation {
            key,
            expected,
            next: Some(kernel(cursor)),
        }];
        for id in commits {
            let key = RefKey::bookmark(name(self.execution), RefName::hex(id.as_bytes()));
            let expected = self
                .history
                .read_ref(&key)
                .map_err(history_error)?
                .map(|v| v.revision);
            refs.push(RefMutation {
                key,
                expected,
                next: Some(kernel(id)),
            });
        }
        self.history
            .write(
                &Transaction {
                    objects,
                    refs,
                    ..Transaction::default()
                },
                |_| Ok(Vec::new()),
            )
            .map_err(history_error)?;
        let (session, loaded) = narrata_history::Session::open(
            &self.history,
            NodeDomain(self.program.clone()),
            name(self.execution),
        )
        .map_err(history_error)?;
        let pins = self.program.pin_state(&loaded.state)?;
        self.session = session;
        self.current = record(loaded);
        self.prefetched.clear();
        self._pins = pins;
        Ok(())
    }
    pub fn view(&self, names: Option<&NameTable>) -> Result<SessionView> {
        let record = &self.current;
        let machine = Machine {
            program: &self.program,
        };
        let product = &self.program.manifest().product;
        let state = &record.state;
        let presentation = items(record.id, self.step(record.id)?.presentation);
        let variables = |values: &BTreeMap<String, crate::Scalar>| -> Vec<VariableView> {
            values
                .iter()
                .map(|(name, value)| VariableView {
                    name: name.clone(),
                    label: None,
                    value: value.view(),
                })
                .collect()
        };
        let history = self
            .commits()?
            .map(|(id, _)| self.load(id))
            .collect::<Result<Vec<_>>>()?
            .iter()
            .map(|record| {
                let program = self.program_at(record.commit.artifact)?;
                let (graph, node, instance, title) = (Machine { program }).title(&record.state)?;
                Ok(HistoryView {
                    id: record.id,
                    parent: record.commit.parent,
                    depth: record.commit.depth,
                    key: names
                        .filter(|names| names.artifact == record.commit.artifact)
                        .and_then(|names| names.node(&graph, &node).map(str::to_owned)),
                    graph,
                    node,
                    instance,
                    title,
                    finished: record.state.finished.is_some(),
                    current: record.id == self.current.id,
                })
            })
            .collect::<Result<_>>()?;
        Ok(SessionView {
            artifact_id: self.program.artifact_id(),
            execution: self.execution,
            cursor: record.id,
            depth: record.commit.depth,
            product: ProductView {
                id: product.id.clone(),
                title: product.title.clone(),
            },
            presentation,
            interaction: machine.interaction(state, names)?,
            shared: state
                .shared
                .iter()
                .map(|(name, value)| VariableView {
                    name: name.clone(),
                    label: product
                        .shared
                        .get(name)
                        .and_then(|variable| variable.label.clone()),
                    value: value.view(),
                })
                .collect(),
            frames: state
                .frames
                .iter()
                .map(|frame| FrameView {
                    graph: frame.graph.clone(),
                    node: frame.node,
                    key: names
                        .and_then(|names| names.node(&frame.graph, &frame.node).map(str::to_owned)),
                    at: frame.at,
                    instance: frame.instance,
                    parameters: variables(&frame.parameters),
                    locals: variables(&frame.locals),
                })
                .collect(),
            history,
        })
    }
}

type Legacy = (ExecutionId, CommitId, Vec<Object>, Vec<CommitId>);

#[cfg(feature = "trace")]
impl<B: StorageBackend> Session<B> {
    /// Read-only assessment. The host selects the complete descriptor path explicitly;
    /// neither registration nor assessment changes an execution's artifact or any Ref.
    pub fn assess_evolution(
        &self,
        registry: &crate::history::evolution::EvolutionRegistry,
        target: ArtifactId,
        migration_ids: &[crate::history::evolution::MigrationId],
        limits: crate::history::evolution::PathLimits,
    ) -> crate::history::evolution::AssessmentV1 {
        use crate::history::evolution::*;
        match self.assess(registry, target, migration_ids, limits) {
            Ok(result) => result,
            Err(error) if matches!(error.code.as_str(), "needs_object" | "needs_artifact") => {
                AssessmentV1::NeedsObjects {
                    digests: vec![ObjectId::from_bytes(
                        error
                            .path
                            .parse::<narrata_history::ObjectId>()
                            .map(|id| *id.as_bytes())
                            .or_else(|_| error.path.parse::<ArtifactId>().map(|id| *id.as_bytes()))
                            .unwrap_or(*target.as_bytes()),
                    )],
                    retryable: true,
                }
            }
            Err(error) => AssessmentV1::InvalidSave {
                diagnostic: evolution_diagnostic(&error),
                restart_available: true,
            },
        }
    }

    fn assess(
        &self,
        registry: &crate::history::evolution::EvolutionRegistry,
        target: ArtifactId,
        migration_ids: &[crate::history::evolution::MigrationId],
        limits: crate::history::evolution::PathLimits,
    ) -> Result<crate::history::evolution::AssessmentV1> {
        use crate::history::evolution::*;
        use narrata_history::migration::{MigrationError, MigrationRequest};
        let key = RefKey::active(name(self.execution))
            .map_err(|e| Error::new("name", "cursor", e.to_string()))?;
        let cursor = self
            .history
            .read_ref(&key)
            .map_err(history_error)?
            .ok_or_else(|| Error::new("invalid_save", "cursor", "missing active cursor"))?;
        let source_header = self.header(node(cursor.commit))?;
        let source_program = registry.program(source_header.artifact)?;
        self.history
            .load(&NodeDomain(source_program.clone()), cursor.commit)
            .map_err(history_error)?;
        let source = SourceV1 {
            source_commit: node(cursor.commit),
            artifact_id: source_header.artifact,
        };
        if source_header.artifact == target {
            return Ok(AssessmentV1::Exact { source });
        }
        registry.program(target)?;
        let path = registry.compare_path(&self.history, source.source_commit, target, limits)?;
        let request = MigrationRequest {
            source: cursor.commit,
            target: artifact(target),
            explicit_path: Some(migration_ids.iter().copied().map(migration).collect()),
            target_ref: key,
            expected_ref: Some(cursor.revision),
            observed_at: 0,
        };
        let prepared = match self.history.dry_run_migration(
            &registry.artifacts,
            &registry.migrations,
            &request,
            |descriptor, from, to, state| descriptor.transform(&from.0, &to.0, state),
        ) {
            Ok(result) => result,
            Err(MigrationError::Step(error)) => {
                return Ok(AssessmentV1::OldVersionOnly {
                    source,
                    target,
                    reason: error.code,
                    restart_available: true,
                });
            }
            Err(MigrationError::Registry(error)) => {
                return Ok(AssessmentV1::OldVersionOnly {
                    source,
                    target,
                    reason: error.to_string(),
                    restart_available: true,
                });
            }
            Err(MigrationError::MissingArtifact(id)) => {
                return Err(Error::new(
                    "needs_artifact",
                    ArtifactId::from_bytes(*id.as_bytes()).to_string(),
                    "missing migration artifact",
                ));
            }
            Err(MigrationError::History(error)) => return Err(history_error(error)),
        };
        let losses = prepared.reports.into_iter().flatten().collect::<Vec<_>>();
        let reason = if !losses.is_empty() {
            Some(ConfirmationReason::Recovery)
        } else {
            match &path {
                PathComparisonV1::Equivalent => None,
                PathComparisonV1::Diverged { .. } => Some(ConfirmationReason::Divergence),
                PathComparisonV1::Unverified { .. } => Some(ConfirmationReason::Unverified),
            }
        };
        let report = MigrationReportV1 {
            from_artifact: source_header.artifact,
            to_artifact: target,
            source_commit: source.source_commit,
            migration_commit: node(prepared.commit),
            losses,
            path,
            external_effects: ExternalEffectsV1::NotApplicable,
        };
        let plan = Box::new(PlanV1 {
            source_commit: source.source_commit,
            target_artifact: target,
            migration_ids: migration_ids.to_vec(),
            expected_cursor_revision: CursorRevision(cursor.revision.get()),
            migration_commit: node(prepared.commit),
            report_digest: report.digest()?,
            report,
        });
        Ok(match reason {
            Some(reason) => AssessmentV1::NeedsConfirmation { plan, reason },
            None => AssessmentV1::Automatic { plan },
        })
    }

    /// Explicit application of a host-approved plan. Reassessment invalidates stale
    /// confirmations; the coordinator repeats planning and atomically CASes only the cursor.
    pub fn apply_evolution(
        &mut self,
        registry: &crate::history::evolution::EvolutionRegistry,
        approved: &crate::history::evolution::PlanV1,
        limits: crate::history::evolution::PathLimits,
    ) -> crate::history::evolution::ApplyV1 {
        use crate::history::evolution::*;
        match self.apply_evolution_checked(registry, approved, limits) {
            Ok(result) => result,
            Err(error) => ApplyV1::Failed {
                diagnostic: evolution_diagnostic(&error),
            },
        }
    }
    fn apply_evolution_checked(
        &mut self,
        registry: &crate::history::evolution::EvolutionRegistry,
        approved: &crate::history::evolution::PlanV1,
        limits: crate::history::evolution::PathLimits,
    ) -> Result<crate::history::evolution::ApplyV1> {
        use crate::history::evolution::*;
        use narrata_history::migration::{MigrationError, MigrationRequest};
        let key = RefKey::active(name(self.execution))
            .map_err(|e| Error::new("name", "cursor", e.to_string()))?;
        let current = self.history.read_ref(&key).map_err(history_error)?;
        let actual = current.map(|v| CursorV1 {
            commit: node(v.commit),
            revision: CursorRevision(v.revision.get()),
        });
        if actual.as_ref().is_none_or(|v| {
            v.commit != approved.source_commit || v.revision != approved.expected_cursor_revision
        }) {
            return Ok(ApplyV1::Conflict {
                actual_cursor: actual,
            });
        }
        let fresh = match self.assess_evolution(
            registry,
            approved.target_artifact,
            &approved.migration_ids,
            limits,
        ) {
            AssessmentV1::Automatic { plan } | AssessmentV1::NeedsConfirmation { plan, .. } => plan,
            _ => return Ok(ApplyV1::PlanChanged),
        };
        if fresh.as_ref() != approved {
            return Ok(ApplyV1::PlanChanged);
        }
        let program = registry.program(approved.target_artifact)?.clone();
        let request = MigrationRequest {
            source: kernel(approved.source_commit),
            target: artifact(approved.target_artifact),
            explicit_path: Some(
                approved
                    .migration_ids
                    .iter()
                    .copied()
                    .map(migration)
                    .collect(),
            ),
            target_ref: key.clone(),
            expected_ref: current.map(|v| v.revision),
            observed_at: 0,
        };
        let result = self.history.apply_migration(
            &registry.artifacts,
            &registry.migrations,
            &request,
            |descriptor, from, to, state| descriptor.transform(&from.0, &to.0, state),
        );
        match result {
            Ok(prepared) => {
                if node(prepared.commit) != approved.migration_commit {
                    return Err(Error::new(
                        "nondeterministic",
                        "migration",
                        "application commit differs from its plan",
                    ));
                }
                self.programs.extend(registry.programs.clone());
                self.reopen(program)?;
                Ok(ApplyV1::Migrated {
                    source_commit: approved.source_commit,
                    migration_commit: approved.migration_commit,
                    target_artifact: approved.target_artifact,
                    report: Box::new(approved.report.clone()),
                })
            }
            Err(MigrationError::History(
                HistoryError::RefConflict(_) | HistoryError::HeadMoved { .. },
            )) => {
                let actual_cursor = self
                    .history
                    .read_ref(&key)
                    .map_err(history_error)?
                    .map(|v| CursorV1 {
                        commit: node(v.commit),
                        revision: CursorRevision(v.revision.get()),
                    });
                Ok(ApplyV1::Conflict { actual_cursor })
            }
            Err(MigrationError::History(error)) => Err(history_error(error)),
            Err(MigrationError::Step(error)) => Err(error),
            Err(other) => Err(Error::new("migration", "apply", other.to_string())),
        }
    }
}

#[cfg(feature = "trace")]
fn evolution_diagnostic(error: &Error) -> crate::Diagnostic {
    // Runtime diagnostics can contain values. Evolution reports reveal only classification
    // and structural location, never private variable values or resolved content.
    crate::Diagnostic {
        code: error.code.clone(),
        path: "evolution".into(),
        message: "artifact evolution could not complete".into(),
    }
}

fn checkpoint_objects(bytes: &[u8]) -> Result<(narrata_history::ObjectId, Vec<Object>)> {
    let allowed = |kind| {
        matches!(
            kind,
            wire::KIND_COMMIT
                | wire::KIND_STATE
                | wire::KIND_INPUT
                | narrata_history::migration::MIGRATION_INPUT_KIND
        )
    };
    if bytes.starts_with(narrata_history::SHALLOW_MAGIC) {
        let bundle =
            narrata_history::ShallowBundle::from_bytes(bytes, BundleLimits::default(), allowed)
                .map_err(|e| Error::new("save", "checkpoint", e.to_string()))?;
        Ok((bundle.manifest.checkpoint.root, bundle.objects))
    } else {
        let bundle =
            narrata_history::CheckpointBundle::from_bytes(bytes, BundleLimits::default(), allowed)
                .map_err(|e| Error::new("save", "checkpoint", e.to_string()))?;
        Ok((bundle.manifest.root, bundle.objects))
    }
}
fn checkpoint_root(bytes: &[u8]) -> Result<narrata_history::ObjectId> {
    checkpoint_objects(bytes).map(|(root, _)| root)
}

fn legacy(program: &Arc<Program>, text: &str) -> Result<Legacy> {
    let export: SessionExport = parse_json_limited(text, MAX_INTERIM_EXPORT_BYTES)?;
    if export.format_version != EXPORT_FORMAT_VERSION {
        return Err(Error::new(
            "version",
            "format_version",
            "unsupported interim export",
        ));
    }
    if export.artifact_id != program.artifact_id() {
        return Err(Error::new(
            "incompatible_save",
            "artifact_id",
            "the export belongs to another artifact",
        ));
    }
    if export.objects.len() > MAX_COMMITS * 3 {
        return Err(Error::new("limit", "objects", "too many interim objects"));
    }
    let mut objects = BTreeMap::new();
    let mut states = BTreeMap::new();
    let mut inputs = BTreeMap::new();
    let mut commits = Vec::new();
    for (position, text) in export.objects.iter().enumerate() {
        let path = format!("objects[{position}]");
        let within = |error: Error| Error {
            path: format!("{path}.{}", error.path),
            ..error
        };
        let bytes = hex::decode(text)
            .map_err(|_| Error::new("decode", &path, "expected hexadecimal envelope"))?;
        let kind = bytes
            .get(10..12)
            .map(|k| u16::from_be_bytes([k[0], k[1]]))
            .ok_or_else(|| Error::new("decode", &path, "truncated envelope"))?;
        let object = Object::from_bytes(&bytes, kind, 1, MAX_EXPORT_BYTES as u64)
            .map_err(|e| Error::new("decode", format!("{path}.envelope"), e.to_string()))?;
        match kind {
            wire::KIND_STATE => {
                let (state, id) = decode_state(program, &bytes).map_err(within)?;
                states.insert(id, state);
            }
            wire::KIND_INPUT => {
                inputs.insert(ObjectId::from_bytes(*object.id().as_bytes()), bytes.clone());
            }
            wire::KIND_COMMIT => {
                let (commit, old) = Commit::decode_interim(&bytes)
                    .or_else(|_| {
                        Commit::decode(&bytes).map(|c| {
                            let id = c.id();
                            (c, id)
                        })
                    })
                    .map_err(within)?;
                commits.push((old, commit));
            }
            _ => return Err(Error::new("decode", &path, "unexpected object kind")),
        }
        if objects.insert(object.id(), object).is_some() {
            return Err(Error::new("duplicate", &path, "duplicate object"));
        }
    }
    if commits.is_empty() || commits.len() > MAX_COMMITS {
        return Err(Error::new(
            "limit",
            "objects",
            "expected 1..512 interim commits",
        ));
    }
    let mut remapped: BTreeMap<CommitId, (Commit, State)> = BTreeMap::new();
    let mut used_states = BTreeSet::new();
    let mut used_inputs = BTreeSet::new();
    let mut ids = Vec::new();
    let mut converted = Vec::new();
    for (position, (old, mut commit)) in commits.into_iter().enumerate() {
        if commit.artifact != program.artifact_id() {
            return Err(Error::new(
                "incompatible_save",
                "commit",
                "commit of another artifact",
            ));
        }
        let state = states
            .get(&commit.state)
            .ok_or_else(|| Error::new("save", "objects", "missing state"))?;
        used_states.insert(commit.state);
        match (commit.parent, commit.input) {
            (None, None) => {
                if position != 0 || (Machine { program }).initial()?.state != *state {
                    return Err(Error::new("save", "root", "invalid initial root"));
                }
            }
            (Some(parent), Some(input)) => {
                let (mapped, parent_state) = remapped.get(&parent).ok_or_else(|| {
                    Error::new("save", "parent", "a parent precedes its children")
                })?;
                if mapped.depth.checked_add(1) != Some(commit.depth) {
                    return Err(Error::new(
                        "save",
                        "depth",
                        "depth must be the parent's plus one",
                    ));
                }
                let bytes = inputs
                    .get(&input)
                    .ok_or_else(|| Error::new("save", "input", "missing input"))?;
                // Validate proposal derivation against the original identity before remapping.
                decode_input(program, &parent, parent_state, bytes)?;
                used_inputs.insert(input);
                commit.parent = Some(mapped.id());
            }
            _ => return Err(Error::new("save", "commit", "invalid commit shape")),
        }
        ids.push(commit.id());
        converted.push(Object::new(wire::KIND_COMMIT, 1, &commit.encode()));
        remapped.insert(old, (commit, state.clone()));
    }
    if used_states.len() != states.len()
        || used_inputs.len() != inputs.len()
        || states.values().map(|s| s.encode().len()).sum::<usize>() > MAX_RETAINED_STATE_BYTES
    {
        return Err(Error::new(
            "save",
            "objects",
            "objects must be referenced and within interim limits",
        ));
    }
    let cursor = remapped
        .get(&export.cursor)
        .ok_or_else(|| Error::new("save", "cursor", "cursor is not retained"))?
        .0
        .id();
    converted.extend(
        objects
            .into_values()
            .filter(|o| o.kind() != wire::KIND_COMMIT),
    );
    Ok((export.execution, cursor, converted, ids))
}

fn items(commit: CommitId, items: Vec<Item>) -> Vec<PresentationItem> {
    items
        .into_iter()
        .enumerate()
        .map(|(occurrence, item)| PresentationItem {
            commit,
            occurrence: occurrence as u32,
            role: item.role,
            node: item.node,
            content: item.content,
            args: item.args,
        })
        .collect()
}
