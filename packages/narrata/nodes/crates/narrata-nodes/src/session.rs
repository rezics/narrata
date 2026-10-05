//! Reference session coordinator (ADR 0013 §6): an in-memory tree of immutable commits.
//! Restoring checks every object and state against the artifact without replaying play;
//! [`Session::verify_path`] replays from the root on request.

use std::{collections::BTreeMap, sync::Arc};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ArtifactId, ChoicePointId, CommitId, Error, ExecutionId, ObjectId, OptionId, Program, Result,
    json::parse_json_limited,
    plan::NameTable,
    runtime::{Item, Machine, Step},
    state::{Commit, Input, State, check_state, decode_input, decode_state},
    view::{FrameView, HistoryView, PresentationItem, ProductView, SessionView, VariableView},
    wire,
};

pub const MAX_COMMITS: usize = 512;
pub const MAX_RETAINED_STATE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_EXPORT_BYTES: usize = 8 * 1024 * 1024;
/// The temporary JSON container's version, until kernel node-session storage replaces it.
pub const EXPORT_FORMAT_VERSION: u16 = 2;

#[derive(Clone, Debug)]
struct Record {
    id: CommitId,
    commit: Commit,
    state: Arc<State>,
    input: Option<Input>,
}

/// Single-process coordinator. A host persists an export before it shows a new view.
#[derive(Clone, Debug)]
pub struct Session {
    program: Arc<Program>,
    execution: ExecutionId,
    records: Vec<Record>,
    index: BTreeMap<CommitId, usize>,
    states: BTreeMap<ObjectId, usize>,
    cursor: usize,
}

/// Temporary session export (ADR 0013 §6). The objects are the same envelopes a kernel store
/// keeps; switching to that store replaces only this container.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionExport {
    pub format_version: u16,
    pub artifact_id: ArtifactId,
    pub execution: ExecutionId,
    pub cursor: CommitId,
    /// Hexadecimal envelopes; each commit follows the input and state it references.
    pub objects: Vec<String>,
}

impl Session {
    pub fn new(program: Arc<Program>, execution: ExecutionId) -> Result<Self> {
        let step = Machine { program: &program }.initial()?;
        let mut session = Self {
            program,
            execution,
            records: Vec::new(),
            index: BTreeMap::new(),
            states: BTreeMap::new(),
            cursor: 0,
        };
        session.insert(None, None, step.state)?;
        Ok(session)
    }

    pub fn program(&self) -> &Arc<Program> {
        &self.program
    }

    pub fn execution(&self) -> ExecutionId {
        self.execution
    }

    fn record(&self, index: usize) -> Result<&Record> {
        self.records
            .get(index)
            .ok_or_else(|| Error::new("state", "cursor", "missing commit"))
    }

    pub fn cursor(&self) -> Result<CommitId> {
        Ok(self.record(self.cursor)?.id)
    }

    pub fn state(&self) -> Result<&State> {
        Ok(&self.record(self.cursor)?.state)
    }

    pub fn commits(&self) -> impl Iterator<Item = (CommitId, &Commit)> {
        self.records
            .iter()
            .map(|record| (record.id, &record.commit))
    }

    /// Adds a commit, or returns the existing one for the same parent and input.
    fn insert(
        &mut self,
        parent: Option<usize>,
        input: Option<Input>,
        state: State,
    ) -> Result<usize> {
        let state_bytes = state.encode().len();
        let state_id = state.id();
        let parent_record = parent.map(|index| self.record(index)).transpose()?;
        let commit = Commit {
            artifact: self.program.artifact_id(),
            parent: parent_record.map(|record| record.id),
            input: input.as_ref().map(Input::id),
            state: state_id,
            depth: parent_record.map_or(Ok(0), |record| {
                record
                    .commit
                    .depth
                    .checked_add(1)
                    .ok_or_else(|| Error::new("limit", "depth", "commit depth overflow"))
            })?,
        };
        let id = commit.id();
        if let Some(index) = self.index.get(&id) {
            return Ok(*index);
        }
        let retained = self.states.values().sum::<usize>()
            + if self.states.contains_key(&state_id) {
                0
            } else {
                state_bytes
            };
        if self.records.len() >= MAX_COMMITS || retained > MAX_RETAINED_STATE_BYTES {
            return Err(Error::new(
                "history_limit",
                "session",
                "retention limit reached (512 commits / 2 MiB of states); export the journey and start a new one",
            ));
        }
        self.states.insert(state_id, state_bytes);
        self.index.insert(id, self.records.len());
        self.records.push(Record {
            id,
            commit,
            state: Arc::new(state),
            input,
        });
        Ok(self.records.len() - 1)
    }

    /// Chooses options at the displayed interaction. A stale `expected` commit, hidden or
    /// disabled options and any failure while running leave the session unchanged.
    pub fn choose(
        &mut self,
        expected: &CommitId,
        choice_point: ChoicePointId,
        options: Vec<OptionId>,
    ) -> Result<CommitId> {
        if self.cursor()? != *expected {
            return Err(Error::new(
                "stale_input",
                "expected_commit",
                "the displayed interaction is no longer current",
            ));
        }
        let input = Input::Choose {
            choice_point,
            options,
        };
        let step = Machine {
            program: &self.program,
        }
        .choose(&self.record(self.cursor)?.state, &input)?;
        self.cursor = self.insert(Some(self.cursor), Some(input), step.state)?;
        self.cursor()
    }

    pub fn checkout(&mut self, commit: &CommitId) -> Result<()> {
        let index = *self.index.get(commit).ok_or_else(|| {
            Error::new(
                "reference",
                "commit",
                "commit is not retained in this session",
            )
        })?;
        // The target's presentation is recomputed before the cursor moves.
        self.step(index)?;
        self.cursor = index;
        Ok(())
    }

    /// Recomputes a commit's step: the root from the manifest, others from the parent state
    /// and the input. The recomputed state must be the recorded one.
    fn step(&self, index: usize) -> Result<Step> {
        let record = self.record(index)?;
        let machine = Machine {
            program: &self.program,
        };
        let step =
            match (&record.commit.parent, &record.input) {
                (Some(parent), Some(input)) => {
                    let parent =
                        self.record(*self.index.get(parent).ok_or_else(|| {
                            Error::new("state", "commit", "missing parent commit")
                        })?)?;
                    machine.choose(&parent.state, input)?
                }
                _ => machine.initial()?,
            };
        if step.state != *record.state {
            return Err(Error::new(
                "unreachable_state",
                record.id.to_string(),
                "the recorded state is not what its parent and input produce",
            ));
        }
        Ok(step)
    }

    pub fn presentation(&self, commit: &CommitId) -> Result<Vec<PresentationItem>> {
        let index = *self
            .index
            .get(commit)
            .ok_or_else(|| Error::new("reference", "commit", "unknown commit"))?;
        Ok(items(*commit, self.step(index)?.presentation))
    }

    /// The reading page: the presentation of the step that entered the current passage (or
    /// ended the story), followed by those of the local choices made in it since.
    pub fn page(&self) -> Result<Vec<PresentationItem>> {
        let mut parts = Vec::new();
        let mut index = self.cursor;
        loop {
            let step = self.step(index)?;
            let record = self.record(index)?;
            let entered = step.entered || record.state.finished.is_some();
            parts.push(items(record.id, step.presentation));
            match (&record.commit.parent, entered) {
                (Some(parent), false) => {
                    index = *self
                        .index
                        .get(parent)
                        .ok_or_else(|| Error::new("state", "commit", "missing parent commit"))?;
                }
                _ => break,
            }
        }
        Ok(parts.into_iter().rev().flatten().collect())
    }

    /// Replays from the root to `commit`, proving that play reaches every state on the path.
    pub fn verify_path(&self, commit: &CommitId) -> Result<()> {
        let mut index = *self
            .index
            .get(commit)
            .ok_or_else(|| Error::new("reference", "commit", "unknown commit"))?;
        let mut path = vec![index];
        while let Some(parent) = &self.record(index)?.commit.parent {
            index = *self
                .index
                .get(parent)
                .ok_or_else(|| Error::new("state", "commit", "missing parent commit"))?;
            path.push(index);
        }
        for index in path.into_iter().rev() {
            self.step(index)?;
        }
        Ok(())
    }

    pub fn view(&self, names: Option<&NameTable>) -> Result<SessionView> {
        let record = self.record(self.cursor)?;
        let machine = Machine {
            program: &self.program,
        };
        let product = &self.program.manifest().product;
        let state = &record.state;
        let presentation = items(record.id, self.step(self.cursor)?.presentation);
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
            .records
            .iter()
            .enumerate()
            .map(|(index, record)| {
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
                    current: index == self.cursor,
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

    pub fn export(&self) -> Result<String> {
        let mut objects = Vec::new();
        let mut written = std::collections::BTreeSet::new();
        for record in &self.records {
            if let Some(input) = &record.input
                && written.insert(input.id())
            {
                objects.push(hex::encode(input.envelope()));
            }
            if written.insert(record.commit.state) {
                objects.push(hex::encode(record.state.envelope()));
            }
            objects.push(hex::encode(record.commit.envelope()));
        }
        let export = SessionExport {
            format_version: EXPORT_FORMAT_VERSION,
            artifact_id: self.program.artifact_id(),
            execution: self.execution,
            cursor: self.cursor()?,
            objects,
        };
        let text = serde_json::to_string(&export)
            .map_err(|error| Error::new("encoding", "export", error.to_string()))?;
        if text.len() > MAX_EXPORT_BYTES {
            return Err(Error::new("limit", "export", "export exceeds 8 MiB"));
        }
        Ok(text)
    }

    /// Restores an export without replaying it: every object's digest is recomputed, the
    /// artifact must match, every state passes `decode_state` and every input passes
    /// `decode_input` against its parent's state. The root state must be the initial one.
    pub fn restore(program: Arc<Program>, text: &str) -> Result<Self> {
        let export: SessionExport = parse_json_limited(text, MAX_EXPORT_BYTES)?;
        if export.format_version != EXPORT_FORMAT_VERSION {
            return Err(Error::new(
                "version",
                "format_version",
                "unsupported session export format",
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
            return Err(Error::new("limit", "objects", "too many objects"));
        }
        let mut states = BTreeMap::new();
        let mut inputs = BTreeMap::new();
        let mut commits = Vec::new();
        for (position, text) in export.objects.iter().enumerate() {
            let path = format!("objects[{position}]");
            // A decoding error names the object it was found in.
            let within = |error: Error| Error {
                path: format!("{path}.{}", error.path),
                ..error
            };
            let bytes = hex::decode(text)
                .map_err(|_| Error::new("decode", &path, "objects are hexadecimal envelopes"))?;
            match bytes
                .get(10..12)
                .map(|kind| u16::from_be_bytes([kind[0], kind[1]]))
            {
                Some(wire::KIND_STATE) => {
                    let (state, id) = decode_state(&program, &bytes).map_err(within)?;
                    if states.insert(id, (state, bytes.len())).is_some() {
                        return Err(Error::new("duplicate", &path, "duplicate object"));
                    }
                }
                Some(wire::KIND_INPUT) => {
                    let id = wire::id_of(wire::KIND_INPUT, bytes.get(56..).unwrap_or_default());
                    if inputs.insert(id, bytes).is_some() {
                        return Err(Error::new("duplicate", &path, "duplicate object"));
                    }
                }
                Some(wire::KIND_COMMIT) => commits.push(Commit::decode(&bytes).map_err(within)?),
                _ => return Err(Error::new("decode", &path, "unexpected object kind")),
            }
        }
        if commits.is_empty() || commits.len() > MAX_COMMITS {
            return Err(Error::new("limit", "objects", "expected 1..512 commits"));
        }
        let mut session = Self {
            program: program.clone(),
            execution: export.execution,
            records: Vec::new(),
            index: BTreeMap::new(),
            states: BTreeMap::new(),
            cursor: 0,
        };
        let mut used_inputs = std::collections::BTreeSet::new();
        for (position, commit) in commits.into_iter().enumerate() {
            let path = format!("commits[{position}]");
            if commit.artifact != program.artifact_id() {
                return Err(Error::new(
                    "incompatible_save",
                    &path,
                    "commit of another artifact",
                ));
            }
            let (state, state_bytes) = states
                .get(&commit.state)
                .cloned()
                .ok_or_else(|| Error::new("save", &path, "missing state object"))?;
            let input = match (&commit.parent, &commit.input) {
                (None, None) => {
                    if position != 0 {
                        return Err(Error::new("save", &path, "the root commit comes first"));
                    }
                    let initial = Machine { program: &program }.initial()?;
                    if initial.state != state {
                        return Err(Error::new(
                            "save",
                            &path,
                            "the root state is not the artifact's initial state",
                        ));
                    }
                    None
                }
                (Some(parent), Some(input_id)) => {
                    let parent_index = *session.index.get(parent).ok_or_else(|| {
                        Error::new("save", &path, "a parent commit precedes its children")
                    })?;
                    let parent = session.record(parent_index)?;
                    if parent.commit.depth.checked_add(1) != Some(commit.depth) {
                        return Err(Error::new(
                            "save",
                            &path,
                            "depth must be the parent's plus one",
                        ));
                    }
                    let bytes = inputs
                        .get(input_id)
                        .ok_or_else(|| Error::new("save", &path, "missing input object"))?;
                    let (input, _) = decode_input(&program, &parent.state, bytes)?;
                    used_inputs.insert(*input_id);
                    Some(input)
                }
                _ => return Err(Error::new("save", &path, "malformed commit")),
            };
            check_state(&program, &state)?;
            let id = commit.id();
            if session.index.insert(id, session.records.len()).is_some() {
                return Err(Error::new("duplicate", &path, "duplicate commit"));
            }
            session.states.insert(commit.state, state_bytes - 56);
            session.records.push(Record {
                id,
                commit,
                state: Arc::new(state),
                input,
            });
        }
        if used_inputs.len() != inputs.len()
            || session.states.len() != states.len()
            || session.states.values().sum::<usize>() > MAX_RETAINED_STATE_BYTES
        {
            return Err(Error::new(
                "save",
                "objects",
                "objects must be referenced and within the retention limits",
            ));
        }
        session.cursor = *session
            .index
            .get(&export.cursor)
            .ok_or_else(|| Error::new("save", "cursor", "cursor is not a retained commit"))?;
        Ok(session)
    }
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
