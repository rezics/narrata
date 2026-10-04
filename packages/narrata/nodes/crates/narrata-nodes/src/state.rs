//! Session objects (ADR 0013 §6): State, Input and Commit. They hold IDs and values only,
//! never text or aliases. `decode_state` and `decode_input` are the only way to accept them
//! from outside; both check the bytes against the artifact.

use std::collections::BTreeMap;

use crate::{
    ArtifactId, ChoicePointId, CommitId, Error, MAX_CALL_DEPTH, MAX_STATE_BYTES, NodeId, ObjectId,
    OptionId, Program, Result, Scalar,
    plan::{CallTarget, GraphRef, Plan},
    wire,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct State {
    pub shared: BTreeMap<String, Scalar>,
    pub frames: Vec<Frame>,
    pub next_instance: u32,
    pub finished: Option<Finished>,
}

/// A graph instance. The top frame of a persisted state waits `at` a choice point of its
/// passage; lower frames wait at the call that created the frame above them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub graph: GraphRef,
    pub node: NodeId,
    pub at: Option<ChoicePointId>,
    pub instance: u32,
    pub parameters: BTreeMap<String, Scalar>,
    pub locals: BTreeMap<String, Scalar>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Finished {
    pub node: NodeId,
    pub instance: u32,
    pub outcome: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Input {
    /// Options in the choice point's order, without repetition.
    Choose {
        choice_point: ChoicePointId,
        options: Vec<OptionId>,
    },
}

/// Matches the kernel node-session commit API: the identity is this payload's object ID and
/// excludes the execution ID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Commit {
    pub artifact: ArtifactId,
    pub parent: Option<CommitId>,
    pub input: Option<ObjectId>,
    pub state: ObjectId,
    pub depth: u64,
}

impl State {
    pub fn encode(&self) -> Vec<u8> {
        wire::encode_state(self)
    }

    pub fn id(&self) -> ObjectId {
        wire::id_of(wire::KIND_STATE, &self.encode())
    }

    pub fn envelope(&self) -> Vec<u8> {
        wire::seal(wire::KIND_STATE, &self.encode()).1
    }
}

impl Input {
    pub fn encode(&self) -> Vec<u8> {
        wire::encode_input(self)
    }

    pub fn id(&self) -> ObjectId {
        wire::id_of(wire::KIND_INPUT, &self.encode())
    }

    pub fn envelope(&self) -> Vec<u8> {
        wire::seal(wire::KIND_INPUT, &self.encode()).1
    }
}

impl Commit {
    pub fn encode(&self) -> Vec<u8> {
        wire::encode_commit(self)
    }

    pub fn id(&self) -> CommitId {
        CommitId::from_bytes(*wire::id_of(wire::KIND_COMMIT, &self.encode()).as_bytes())
    }

    pub fn envelope(&self) -> Vec<u8> {
        wire::seal(wire::KIND_COMMIT, &self.encode()).1
    }

    pub fn decode(envelope: &[u8]) -> Result<Self> {
        let (commit, _) = wire::open(
            envelope,
            wire::KIND_COMMIT,
            1024,
            "commit",
            wire::decode_commit,
            wire::encode_commit,
        )?;
        if commit.parent.is_some() != commit.input.is_some()
            || commit.parent.is_some() != (commit.depth > 0)
        {
            return Err(Error::new(
                "commit",
                "commit",
                "a root commit has neither parent nor input and depth 0; others have both",
            ));
        }
        Ok(commit)
    }
}

pub(crate) fn check_size(state: &State) -> Result<usize> {
    let size = state.encode().len();
    if size > MAX_STATE_BYTES {
        return Err(Error::new(
            "limit",
            "state",
            "logical state exceeds 128 KiB",
        ));
    }
    Ok(size)
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new("invalid_state", "state", message)
}

/// Decodes a State envelope and checks it against the artifact. This proves the state is
/// well formed for the artifact, not that play can reach it; see `Session::verify_path`.
pub fn decode_state(program: &Program, envelope: &[u8]) -> Result<(State, ObjectId)> {
    let (state, id) = wire::open(
        envelope,
        wire::KIND_STATE,
        MAX_STATE_BYTES,
        "state",
        wire::decode_state,
        wire::encode_state,
    )?;
    check_state(program, &state)?;
    Ok((state, id))
}

pub(crate) fn check_state(program: &Program, state: &State) -> Result<()> {
    let product = &program.manifest().product;
    if state.shared.len() != product.shared.len()
        || state
            .shared
            .iter()
            .zip(&product.shared)
            .any(|((name, value), (declared, variable))| {
                name != declared || value.kind() != variable.value.kind()
            })
    {
        return Err(invalid(
            "shared variables differ from the product declaration",
        ));
    }
    if let Some(finished) = &state.finished {
        if !state.frames.is_empty() {
            return Err(invalid("a finished state has no frames"));
        }
        let graph = program.graph(&product.entry)?;
        match graph.nodes.get(&finished.node) {
            Some(Plan::Return { outcome }) if outcome == &finished.outcome => {}
            _ => return Err(invalid("the finishing node does not return that outcome")),
        }
        if finished.instance == 0 || finished.instance >= state.next_instance {
            return Err(invalid("instance numbers must be below next_instance"));
        }
        return Ok(());
    }
    if state.frames.is_empty() || state.frames.len() > MAX_CALL_DEPTH {
        return Err(invalid("an unfinished state has 1..64 frames"));
    }
    let mut previous_instance = 0;
    for (index, frame) in state.frames.iter().enumerate() {
        if frame.instance <= previous_instance || frame.instance >= state.next_instance {
            return Err(invalid(
                "instances increase along the stack and stay below next_instance",
            ));
        }
        previous_instance = frame.instance;
        let graph = program.graph(&frame.graph)?;
        let header = &graph.header;
        if frame.parameters.len() != header.parameters.len()
            || frame
                .parameters
                .iter()
                .zip(&header.parameters)
                .any(|((name, value), (declared, kind))| name != declared || value.kind() != *kind)
        {
            return Err(invalid(
                "frame parameters differ from the graph declaration",
            ));
        }
        if frame.locals.len() != header.locals.len()
            || frame.locals.iter().zip(&header.locals).any(
                |((name, value), (declared, initial))| {
                    name != declared || value.kind() != initial.kind()
                },
            )
        {
            return Err(invalid("frame locals differ from the graph declaration"));
        }
        let plan = graph
            .nodes
            .get(&frame.node)
            .ok_or_else(|| invalid(format!("unknown node {}", frame.node)))?;
        match state.frames.get(index + 1) {
            None => {
                let Plan::Passage(passage) = plan else {
                    return Err(invalid("the top frame waits at a passage"));
                };
                let at = frame
                    .at
                    .ok_or_else(|| invalid("the top frame waits at a choice point"))?;
                if !passage.choice_points.iter().any(|point| point.id == at) {
                    return Err(invalid(format!("unknown choice point {at}")));
                }
            }
            Some(above) => {
                let Plan::Call { target, .. } = plan else {
                    return Err(invalid("a lower frame waits at a call"));
                };
                if frame.at.is_some() || program.resolve_call(&frame.graph, target)? != above.graph
                {
                    return Err(invalid("the call does not lead to the frame above it"));
                }
            }
        }
        if index == 0 && (frame.graph != product.entry || frame.parameters != product.arguments) {
            return Err(invalid(
                "the root frame runs the entry graph with the product arguments",
            ));
        }
    }
    Ok(())
}

/// Decodes an Input envelope and checks it against the choice point its parent waits at.
pub fn decode_input(
    program: &Program,
    parent: &State,
    envelope: &[u8],
) -> Result<(Input, ObjectId)> {
    let (input, id) = wire::open(
        envelope,
        wire::KIND_INPUT,
        8 * 1024,
        "input",
        wire::decode_input,
        wire::encode_input,
    )?;
    check_input(program, parent, &input)?;
    Ok((input, id))
}

/// Returns the indices of the chosen options within their choice point.
pub(crate) fn check_input(program: &Program, parent: &State, input: &Input) -> Result<Vec<usize>> {
    let Input::Choose {
        choice_point,
        options,
    } = input;
    let frame = parent
        .frames
        .last()
        .ok_or_else(|| Error::new("finished", "input", "the story has ended"))?;
    if frame.at != Some(*choice_point) {
        return Err(Error::new(
            "action",
            "input",
            "the choice point is not the current interaction",
        ));
    }
    let graph = program.graph(&frame.graph)?;
    let Some(Plan::Passage(passage)) = graph.nodes.get(&frame.node) else {
        return Err(Error::new(
            "state",
            "input",
            "the current node is not a passage",
        ));
    };
    let point = passage
        .choice_points
        .iter()
        .find(|point| point.id == *choice_point)
        .ok_or_else(|| Error::new("state", "input", "missing current choice point"))?;
    let mut indices = Vec::with_capacity(options.len());
    for option in options {
        let index = point
            .options
            .iter()
            .position(|candidate| candidate.id == *option)
            .ok_or_else(|| Error::new("action", "input", format!("unknown option {option}")))?;
        if indices.last().is_some_and(|last| index <= *last) {
            return Err(Error::new(
                "action",
                "input",
                "options follow the choice point's order without repetition",
            ));
        }
        indices.push(index);
    }
    if indices.len() < usize::from(point.min) || indices.len() > usize::from(point.max) {
        return Err(Error::new(
            "cardinality",
            "input",
            format!("choose {}..={} options", point.min, point.max),
        ));
    }
    Ok(indices)
}

/// Resolves a call target of `graph` to the graph it runs.
pub(crate) fn resolve_call(
    manifest: &crate::plan::Manifest,
    graph: &GraphRef,
    target: &CallTarget,
) -> Result<GraphRef> {
    match target {
        CallTarget::Local { graph: name } => Ok(GraphRef {
            package: graph.package.clone(),
            graph: name.clone(),
        }),
        CallTarget::Import { port } => manifest
            .product
            .bindings
            .get(&crate::plan::ImportRef {
                package: graph.package.clone(),
                graph: graph.graph.clone(),
                port: port.clone(),
            })
            .cloned()
            .ok_or_else(|| Error::new("binding", graph.label(), format!("unbound import {port}"))),
    }
}
