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
pub const MAX_EXPORT_BYTES: usize = 8 * 1024 * 1024;
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
    execution: ExecutionId,
    history: History<B, DomainKinds<NodeDomain>>,
    session: narrata_history::Session<NodeDomain>,
    current: Record,
    _pins: ChunkPins,
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
            return Err(Error::new("limit", "export", "checkpoint exceeds 8 MiB"));
        }
        let bytes = hex::decode(text.trim())
            .map_err(|e| Error::new("decode", "checkpoint", e.to_string()))?;
        let bundle = narrata_history::CheckpointBundle::from_bytes(
            &bytes,
            BundleLimits::default(),
            |kind| {
                matches!(
                    kind,
                    wire::KIND_COMMIT | wire::KIND_STATE | wire::KIND_INPUT
                )
            },
        )
        .map_err(|e| Error::new("save", "checkpoint", e.to_string()))?;
        let mut id = [0; 16];
        id.copy_from_slice(&bundle.manifest.root.as_bytes()[..16]);
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
    /// Reopens this execution's cursor without replay; creates its root only when absent.
    pub fn open(program: Arc<Program>, execution: ExecutionId, backend: B) -> Result<Self> {
        let mut history =
            History::open(backend, DomainKinds::<NodeDomain>::new()).map_err(history_error)?;
        let domain = NodeDomain(program.clone());
        let mut initial_pins = None;
        let (session, loaded) =
            match narrata_history::Session::open(&history, domain.clone(), name(execution)) {
                Err(HistoryError::MissingRef(_)) => {
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
            program,
            execution,
            history,
            session,
            current: record(loaded),
            _pins: pins,
        })
    }
    pub fn program(&self) -> &Arc<Program> {
        &self.program
    }
    pub fn execution(&self) -> ExecutionId {
        self.execution
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
    fn load(&self, id: CommitId) -> Result<Record> {
        self.history
            .load(self.session.domain(), kernel(id))
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
        let pins = self.program.pin_state(&self.load(*commit)?.state)?;
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
        let pins = self
            .program
            .pin_state(&self.load(node(target.commit))?.state)?;
        let loaded = self
            .session
            .load_save(&mut self.history, &key, 0)
            .map_err(history_error)?;
        self.current = record(loaded);
        self._pins = pins;
        Ok(())
    }
    fn step(&self, id: CommitId) -> Result<Step> {
        let record = self.load(id)?;
        let machine = Machine {
            program: &self.program,
        };
        let step = match (record.commit.parent, record.commit.input) {
            (Some(parent), Some(input)) => {
                let parent = self.load(parent)?;
                let object = self
                    .history
                    .reader()
                    .require(
                        narrata_history::ObjectId::from_bytes(*input.as_bytes()),
                        Some(wire::KIND_INPUT),
                    )
                    .map_err(history_error)?;
                let input_parent = self.input_parent(parent.id, &parent.state, object.bytes())?;
                let (input, _) =
                    decode_input(&self.program, &input_parent, &parent.state, object.bytes())?;
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
        match decode_input(&self.program, &id, state, bytes) {
            Ok(_) => Ok(id),
            Err(error) if error.code == "derived_id" => {
                let old = self.interim_id(id)?;
                decode_input(&self.program, &old, state, bytes)?;
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
        let bytes = bundle
            .to_bytes()
            .map_err(|e| Error::new("save", "checkpoint", e.to_string()))?;
        if bytes.len() > MAX_EXPORT_BYTES / 2 {
            return Err(Error::new(
                "limit",
                "export",
                "checkpoint exceeds 8 MiB of hex",
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
        let restored = Session::restore(self.program.clone(), text)?;
        let bytes = hex::decode(restored.export()?)
            .map_err(|e| Error::new("decode", "checkpoint", e.to_string()))?;
        let key = RefKey::active(name(self.execution))
            .map_err(|e| Error::new("name", "session", e.to_string()))?;
        let expected = self
            .history
            .read_ref(&key)
            .map_err(history_error)?
            .map(|v| v.revision);
        self.history
            .import(
                self.session.domain(),
                &bytes,
                BundleLimits::default(),
                key,
                expected,
                0,
            )
            .map_err(bundle_error)?;
        let (session, loaded) = narrata_history::Session::open(
            &self.history,
            NodeDomain(self.program.clone()),
            name(self.execution),
        )
        .map_err(history_error)?;
        let pins = self.program.pin_state(&loaded.state)?;
        self.session = session;
        self.current = record(loaded);
        self._pins = pins;
        Ok(())
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
        self.import_objects(objects.into_values().collect(), commits, source.current.id)
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
                let (graph, node, instance, title) = machine.title(&record.state)?;
                Ok(HistoryView {
                    id: record.id,
                    parent: record.commit.parent,
                    depth: record.commit.depth,
                    key: names.and_then(|names| names.node(&graph, &node).map(str::to_owned)),
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

fn legacy(program: &Arc<Program>, text: &str) -> Result<Legacy> {
    let export: SessionExport = parse_json_limited(text, MAX_EXPORT_BYTES)?;
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
