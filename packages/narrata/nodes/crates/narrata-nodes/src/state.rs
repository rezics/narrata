//! Session objects (ADR 0013 §6): State, Input and Commit. They hold IDs and values only,
//! never text or aliases. `decode_state` and `decode_input` are the only way to accept them
//! from outside; both check the bytes against the artifact.

use std::{borrow::Cow, collections::BTreeMap};

use narrata_kernel::codec::{EnvelopeLimits, inspect_envelope};

use crate::{
    ArtifactId, ChoicePointId, CommitId, Error, MAX_CALL_DEPTH, MAX_STATE_BYTES, NodeId, ObjectId,
    OptionId, Program, Result, Scalar,
    plan::{CallTarget, Graph, GraphRef, MediaKind, OptionPlan, Passage, Plan, SceneTarget},
    proposal::{apply_proposal, check_overlay},
    wire,
};

/// A propose input carries at most what a state can hold.
pub(crate) const MAX_INPUT_BYTES: usize = MAX_STATE_BYTES;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct State {
    pub shared: BTreeMap<String, Scalar>,
    pub frames: Vec<Frame>,
    pub next_instance: u32,
    pub finished: Option<Finished>,
    /// Present for every VN state, including an initially empty scene. Its object schema
    /// selects the VN state payload while `None` preserves the Gamebook schema-1 bytes.
    pub scene: Option<SceneTarget>,
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
    /// What the host proposed while the frame ran; it vanishes when the frame returns.
    pub overlay: Overlay,
}

/// Structure proposed in one frame (ADR 0013 §5): passages addressed like the graph's own
/// nodes, and options appended to choice points that accept proposals, after the choice
/// point's own options and in proposal order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Overlay {
    pub nodes: BTreeMap<NodeId, Passage>,
    pub options: BTreeMap<ChoicePointId, Vec<OptionPlan>>,
}

impl Overlay {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.options.is_empty()
    }

    /// `node` as a frame with this overlay sees it: a node of `graph` or a proposed passage,
    /// with the options proposed at its choice points appended.
    pub(crate) fn plan<'g>(&self, graph: &'g Graph, node: &NodeId) -> Option<Cow<'g, Plan>> {
        let plan = match graph.nodes.get(node) {
            Some(plan) => Cow::Borrowed(plan),
            None => Cow::Owned(Plan::Passage(self.nodes.get(node)?.clone())),
        };
        match &*plan {
            Plan::Passage(passage)
                if passage
                    .choice_points
                    .iter()
                    .any(|point| self.options.contains_key(&point.id)) =>
            {
                Some(Cow::Owned(Plan::Passage(self.compose(passage))))
            }
            _ => Some(plan),
        }
    }

    /// The passage at `node`, if the node is one.
    pub(crate) fn passage<'g>(&self, graph: &'g Graph, node: &NodeId) -> Option<Cow<'g, Passage>> {
        match self.plan(graph, node)? {
            Cow::Borrowed(Plan::Passage(passage)) => Some(Cow::Borrowed(passage)),
            Cow::Owned(Plan::Passage(passage)) => Some(Cow::Owned(passage)),
            _ => None,
        }
    }

    /// `passage` with the options proposed at its choice points appended.
    pub(crate) fn compose(&self, passage: &Passage) -> Passage {
        let mut passage = passage.clone();
        for point in &mut passage.choice_points {
            if let Some(options) = self.options.get(&point.id) {
                point.options.extend(options.iter().cloned());
            }
        }
        passage
    }
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
    /// A host proposal at the choice point the parent waits at, checked and recorded with
    /// derived IDs (ADR 0013 §5). New nodes keep the request's order, which the derivation
    /// follows.
    Propose {
        choice_point: ChoicePointId,
        options: Vec<OptionPlan>,
        nodes: Vec<(NodeId, Passage)>,
    },
    /// Advance exactly the dialogue node currently shown by the parent state.
    Advance { dialogue: NodeId },
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
        wire::encode_state_schema(self, self.schema())
    }

    pub(crate) fn schema(&self) -> u16 {
        if self.scene.is_some() {
            wire::VISUAL_NOVEL_SCHEMA
        } else {
            wire::SCHEMA
        }
    }

    pub fn id(&self) -> ObjectId {
        wire::id_of_schema(wire::KIND_STATE, self.schema(), &self.encode())
    }

    pub fn envelope(&self) -> Vec<u8> {
        wire::seal_schema(wire::KIND_STATE, self.schema(), &self.encode()).1
    }
}

impl Input {
    pub fn encode(&self) -> Vec<u8> {
        wire::encode_input_schema(self, self.schema())
    }

    pub(crate) fn schema(&self) -> u16 {
        if matches!(self, Self::Advance { .. }) {
            wire::VISUAL_NOVEL_SCHEMA
        } else {
            wire::SCHEMA
        }
    }

    pub fn id(&self) -> ObjectId {
        wire::id_of_schema(wire::KIND_INPUT, self.schema(), &self.encode())
    }

    pub fn envelope(&self) -> Vec<u8> {
        wire::seal_schema(wire::KIND_INPUT, self.schema(), &self.encode()).1
    }
}

impl Commit {
    /// Checked read-only decoding of the interim optional-field commit encoding.
    pub fn decode_interim(envelope: &[u8]) -> Result<(Self, CommitId)> {
        let (commit, id) = wire::open(
            envelope,
            wire::KIND_COMMIT,
            1024,
            "commit",
            wire::decode_interim_commit,
            wire::encode_interim_commit,
        )?;
        if commit.parent.is_some() != commit.input.is_some()
            || commit.parent.is_some() != (commit.depth > 0)
        {
            return Err(Error::new("commit", "commit", "invalid interim root shape"));
        }
        Ok((commit, CommitId::from_bytes(*id.as_bytes())))
    }
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
    let limits = EnvelopeLimits {
        max_envelope_bytes: MAX_STATE_BYTES as u64 + 56,
        max_payload_bytes: MAX_STATE_BYTES as u64,
    };
    let inspected = inspect_envelope(envelope, &limits)
        .map_err(|error| Error::new("decode", "state", error.to_string()))?;
    let schema = inspected.schema_version;
    let (state, id) = match schema {
        wire::SCHEMA if !program.is_visual_novel() => wire::open_schema(
            envelope,
            wire::KIND_STATE,
            schema,
            MAX_STATE_BYTES,
            "state",
            wire::decode_state_schema1,
            wire::encode_state_schema1,
        )?,
        wire::VISUAL_NOVEL_SCHEMA if program.is_visual_novel() => wire::open_schema(
            envelope,
            wire::KIND_STATE,
            schema,
            MAX_STATE_BYTES,
            "state",
            wire::decode_state_schema2,
            wire::encode_state_schema2,
        )?,
        _ => {
            return Err(Error::new(
                "decode",
                "state",
                "state schema does not match artifact",
            ));
        }
    };
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
    if state.scene.is_some() != program.is_visual_novel() {
        return Err(invalid("scene state must match the artifact dialect"));
    }
    if let Some(scene) = &state.scene {
        validate_scene_state(scene)?;
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
        check_overlay(program, &graph, frame)
            .map_err(|error| invalid(format!("overlay: {error}")))?;
        let plan = frame
            .overlay
            .plan(&graph, &frame.node)
            .ok_or_else(|| invalid(format!("unknown node {}", frame.node)))?;
        match state.frames.get(index + 1) {
            None => match &*plan {
                Plan::Passage(passage) => {
                    let at = frame
                        .at
                        .ok_or_else(|| invalid("the top frame waits at a choice point"))?;
                    if !passage.choice_points.iter().any(|point| point.id == at) {
                        return Err(invalid(format!("unknown choice point {at}")));
                    }
                }
                Plan::Dialogue { .. } if frame.at.is_none() => {}
                _ => return Err(invalid("the top frame waits at a valid interaction")),
            },
            Some(above) => {
                let Plan::Call { target, .. } = &*plan else {
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

/// Decodes an Input envelope and checks it against the choice point that `parent`, the
/// state of commit `parent_commit`, waits at. A proposal's derived IDs depend on that commit.
pub fn decode_input(
    program: &Program,
    parent_commit: &CommitId,
    parent: &State,
    envelope: &[u8],
) -> Result<(Input, ObjectId)> {
    let limits = EnvelopeLimits {
        max_envelope_bytes: MAX_INPUT_BYTES as u64 + 56,
        max_payload_bytes: MAX_INPUT_BYTES as u64,
    };
    let inspected = inspect_envelope(envelope, &limits)
        .map_err(|error| Error::new("decode", "input", error.to_string()))?;
    let schema = inspected.schema_version;
    let (input, id) = match schema {
        wire::SCHEMA => wire::open_schema(
            envelope,
            wire::KIND_INPUT,
            schema,
            MAX_INPUT_BYTES,
            "input",
            wire::decode_input_schema1,
            wire::encode_input_schema1,
        )?,
        wire::VISUAL_NOVEL_SCHEMA if program.is_visual_novel() => wire::open_schema(
            envelope,
            wire::KIND_INPUT,
            schema,
            MAX_INPUT_BYTES,
            "input",
            wire::decode_input_schema2,
            |input| wire::encode_input_schema(input, schema),
        )?,
        _ => return Err(Error::new("decode", "input", "unsupported input schema")),
    };
    if input.schema() != schema {
        return Err(Error::new(
            "decode",
            "input",
            "input schema does not match its payload tag",
        ));
    }
    match &input {
        Input::Choose {
            choice_point,
            options,
        } => {
            check_choice(program, parent, choice_point, options)?;
        }
        Input::Propose { .. } => {
            apply_proposal(program, parent_commit, parent, &input)?;
        }
        Input::Advance { dialogue } => check_advance(program, parent, dialogue)?,
    }
    Ok((input, id))
}

pub(crate) fn check_advance(program: &Program, parent: &State, dialogue: &NodeId) -> Result<()> {
    if !program.is_visual_novel() {
        return Err(Error::new(
            "action",
            "input",
            "advance requires a VN artifact",
        ));
    }
    let frame = parent
        .frames
        .last()
        .ok_or_else(|| Error::new("finished", "input", "the story has ended"))?;
    if frame.at.is_some() || frame.node != *dialogue {
        return Err(Error::new(
            "action",
            "input",
            "the dialogue identity is not the current interaction",
        ));
    }
    let graph = program.graph(&frame.graph)?;
    if !matches!(graph.nodes.get(&frame.node), Some(Plan::Dialogue { .. })) {
        return Err(Error::new(
            "action",
            "input",
            "the current interaction is not dialogue",
        ));
    }
    Ok(())
}

fn validate_scene_state(scene: &SceneTarget) -> Result<()> {
    if let Some(background) = &scene.background {
        crate::check::validate_media_ref(background, MediaKind::Image, "state.scene.background")
            .map_err(|error| invalid(error.to_string()))?;
    }
    crate::check::validate_layers(&scene.layers, "state.scene")
        .map_err(|error| invalid(error.to_string()))?;
    if let Some(music) = &scene.music {
        crate::check::validate_media_ref(music, MediaKind::Audio, "state.scene.music")
            .map_err(|error| invalid(error.to_string()))?;
    }
    Ok(())
}

/// Returns the indices of the chosen options within their choice point.
pub(crate) fn check_choice(
    program: &Program,
    parent: &State,
    choice_point: &ChoicePointId,
    options: &[OptionId],
) -> Result<Vec<usize>> {
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
    let passage = frame
        .overlay
        .passage(&graph, &frame.node)
        .ok_or_else(|| Error::new("state", "input", "the current node is not a passage"))?;
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
