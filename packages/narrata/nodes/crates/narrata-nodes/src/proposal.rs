//! Host proposals (ADR 0013 §5). At a choice point that accepts proposals, a generative system
//! or the host proposes options and passages without IDs. The engine checks them with the
//! compile-time rules, derives their IDs from the parent commit and the request, and records
//! them as a `propose` input; rewinding, restoring and migrating use that record and never
//! regenerate it.

use std::collections::{BTreeMap, BTreeSet};

use narrata_kernel::{
    codec::digest_bytes,
    content::{AnchorId, ContentRef, Segment},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    Assignment, ChoicePointId, CommitId, Error, Expr, NodeId, OptionId, Program, Result,
    canonical_json,
    check::{check_passage, name},
    plan::{ChoicePoint, Graph, OptionPlan, Outcome, Passage, Plan, TombstoneSet, node_path},
    source::OutcomeSource,
    state::{Frame, Input, Overlay, State, check_size},
};

pub const MAX_PROPOSED_OPTIONS: usize = 16;
pub const MAX_PROPOSED_NODES: usize = 16;
/// Proposed passages that one frame holds.
pub const MAX_OVERLAY_NODES: usize = 256;
/// The JSON text of one proposal request.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;

const REQUEST_DOMAIN: &str = "narrata.nodes.proposal-request";
const ID_DOMAIN: &str = "narrata.nodes.proposal-id";

/// What a host proposes at the choice point the session waits at. It carries no IDs: existing
/// nodes are named by `node:` ID and proposed ones by their key in this request.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalRequest {
    /// The current choice point; it must accept proposals.
    pub choice_point: ChoicePointId,
    /// Appended after the choice point's options, in this order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<OptionProposal>,
    /// Passages added to the current frame; they vanish when the frame returns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<NodeProposal>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionProposal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<ContentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_if: Option<Expr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_if: Option<Expr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<ContentRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Assignment>,
    /// A branch target is a `node:` ID of the graph or of the frame's earlier proposals, or
    /// the key of a node in this request.
    pub outcome: OutcomeSource,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeProposal {
    /// Names the node within this request only; it is never recorded.
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<ContentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Segment>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, Expr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choice_points: Vec<ChoicePointProposal>,
    /// A `node:` ID or the key of a node in this request, as for branch targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChoicePointProposal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<AnchorId>,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub min: u16,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub max: u16,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub proposals: bool,
    pub options: Vec<OptionProposal>,
}

fn one() -> u16 {
    1
}

fn is_one(value: &u16) -> bool {
    *value == 1
}

/// `digest_bytes("narrata.nodes.proposal-id", 1, parent ‖ request digest ‖ u16 sequence)`
/// cut to 16 bytes and marked as a UUIDv8 (RFC 9562), which tool-minted UUIDv7 IDs never are.
fn derive_id(parent: &CommitId, request: &[u8; 32], sequence: u16) -> [u8; 16] {
    let mut preimage = Vec::with_capacity(66);
    preimage.extend_from_slice(parent.as_bytes());
    preimage.extend_from_slice(request);
    preimage.extend_from_slice(&sequence.to_be_bytes());
    let digest = digest_bytes(ID_DOMAIN, 1, &preimage);
    let mut id = [0_u8; 16];
    id.copy_from_slice(&digest[..16]);
    id[6] = (id[6] & 0x0f) | 0x80;
    id[8] = (id[8] & 0x3f) | 0x80;
    id
}

/// Hands out derived IDs in sequence order.
struct Derivation<'a> {
    parent: &'a CommitId,
    request: [u8; 32],
    sequence: u32,
}

impl Derivation<'_> {
    fn next(&mut self) -> Result<[u8; 16]> {
        let sequence = u16::try_from(self.sequence)
            .map_err(|_| Error::new("limit", "proposal", "a proposal derives at most 65536 IDs"))?;
        self.sequence += 1;
        Ok(derive_id(self.parent, &self.request, sequence))
    }
}

fn keys(request: &ProposalRequest) -> Result<BTreeMap<&str, usize>> {
    let mut keys = BTreeMap::new();
    for (index, node) in request.nodes.iter().enumerate() {
        let path = format!("proposal.nodes[{index}].key");
        name(&node.key, &path)?;
        if keys.insert(node.key.as_str(), index).is_some() {
            return Err(Error::new(
                "duplicate",
                &path,
                format!("duplicate node key {}", node.key),
            ));
        }
    }
    Ok(keys)
}

/// A node named in a request: one that exists, or the one at a position of the request.
#[derive(Clone, Copy)]
enum Target {
    Node(NodeId),
    New(usize),
}

fn target(text: &str, keys: &BTreeMap<&str, usize>, path: &str) -> Result<Target> {
    // Keys are identifiers, which never contain the colon of an ID.
    if text.starts_with("node:") {
        return text
            .parse()
            .map(Target::Node)
            .map_err(|_| Error::new("identifier", path, format!("{text:?} is not a node ID")));
    }
    keys.get(text).copied().map(Target::New).ok_or_else(|| {
        Error::new(
            "reference",
            path,
            format!("no node of this request has the key {text}"),
        )
    })
}

type Rewrite<'a> = dyn FnMut(&str, &str) -> Result<String> + 'a;

fn rewrite_option(
    option: &mut OptionProposal,
    path: &str,
    rewrite: &mut Rewrite<'_>,
) -> Result<()> {
    if let OutcomeSource::Branch { target } = &mut option.outcome {
        *target = rewrite(target, &format!("{path}.outcome.target"))?;
    }
    Ok(())
}

/// Rewrites every node reference of a request; option outcomes and `next` are its only ones.
fn map_targets(request: &ProposalRequest, rewrite: &mut Rewrite<'_>) -> Result<ProposalRequest> {
    let mut request = request.clone();
    for (index, option) in request.options.iter_mut().enumerate() {
        rewrite_option(option, &format!("proposal.options[{index}]"), rewrite)?;
    }
    for (index, node) in request.nodes.iter_mut().enumerate() {
        let path = format!("proposal.nodes[{index}]");
        for (point_index, point) in node.choice_points.iter_mut().enumerate() {
            for (option_index, option) in point.options.iter_mut().enumerate() {
                let path = format!("{path}.choice_points[{point_index}].options[{option_index}]");
                rewrite_option(option, &path, rewrite)?;
            }
        }
        if let Some(next) = &mut node.next {
            *next = rewrite(next, &format!("{path}.next"))?;
        }
    }
    Ok(request)
}

/// The canonical request: node keys become positions and IDs their canonical text, so the
/// names a host picks never reach the derived IDs.
fn normalize(request: &ProposalRequest) -> Result<ProposalRequest> {
    let keys = keys(request)?;
    let mut request = map_targets(request, &mut |text, path| {
        Ok(match target(text, &keys, path)? {
            Target::Node(id) => id.to_string(),
            Target::New(index) => index.to_string(),
        })
    })?;
    for (index, node) in request.nodes.iter_mut().enumerate() {
        node.key = index.to_string();
    }
    Ok(request)
}

fn option_plan(
    option: &OptionProposal,
    id: OptionId,
    resolve: &dyn Fn(&str) -> Result<NodeId>,
) -> Result<OptionPlan> {
    Ok(OptionPlan {
        id,
        label: option.label.clone(),
        visible_if: option.visible_if.clone(),
        enabled_if: option.enabled_if.clone(),
        reason: option.reason.clone(),
        effects: option.effects.clone(),
        outcome: match &option.outcome {
            OutcomeSource::Local { reply, rejoin } => Outcome::Local {
                reply: reply.clone(),
                rejoin: rejoin.clone(),
            },
            OutcomeSource::Branch { target } => Outcome::Branch {
                target: resolve(target)?,
            },
        },
    })
}

/// Derives the IDs of a request proposed after `parent` and records it. Sequence numbers
/// follow one traversal: the appended options, then each node, its choice points and their
/// options, all in request order.
pub(crate) fn materialize(parent: &CommitId, request: &ProposalRequest) -> Result<Input> {
    if request.options.len() > MAX_PROPOSED_OPTIONS || request.nodes.len() > MAX_PROPOSED_NODES {
        return Err(Error::new(
            "limit",
            "proposal",
            "a proposal has at most 16 options and 16 nodes",
        ));
    }
    if request.options.is_empty() && request.nodes.is_empty() {
        return Err(Error::new(
            "proposal",
            "proposal",
            "a proposal adds at least one option or node",
        ));
    }
    let request = normalize(request)?;
    let mut ids = Derivation {
        parent,
        request: digest_bytes(REQUEST_DOMAIN, 1, &canonical_json(&request)?),
        sequence: 0,
    };
    let option_ids = request
        .options
        .iter()
        .map(|_| ids.next().map(OptionId::from_bytes))
        .collect::<Result<Vec<_>>>()?;
    let mut node_ids = Vec::with_capacity(request.nodes.len());
    for node in &request.nodes {
        let id = NodeId::from_bytes(ids.next()?);
        let mut points = Vec::with_capacity(node.choice_points.len());
        for point in &node.choice_points {
            let point_id = ChoicePointId::from_bytes(ids.next()?);
            let options = point
                .options
                .iter()
                .map(|_| ids.next().map(OptionId::from_bytes))
                .collect::<Result<Vec<_>>>()?;
            points.push((point_id, options));
        }
        node_ids.push((id, points));
    }
    let keys = keys(&request)?;
    let resolve = |text: &str| -> Result<NodeId> {
        Ok(match target(text, &keys, "proposal")? {
            Target::Node(id) => id,
            Target::New(index) => node_ids[index].0,
        })
    };
    let options = request
        .options
        .iter()
        .zip(option_ids)
        .map(|(option, id)| option_plan(option, id, &resolve))
        .collect::<Result<_>>()?;
    let mut nodes = Vec::with_capacity(request.nodes.len());
    for (node, (id, points)) in request.nodes.iter().zip(&node_ids) {
        let mut choice_points = Vec::with_capacity(points.len());
        for (point, (point_id, option_ids)) in node.choice_points.iter().zip(points) {
            choice_points.push(ChoicePoint {
                id: *point_id,
                placement: point.placement.clone(),
                min: point.min,
                max: point.max,
                options: point
                    .options
                    .iter()
                    .zip(option_ids)
                    .map(|(option, id)| option_plan(option, *id, &resolve))
                    .collect::<Result<_>>()?,
                proposals: point.proposals,
            });
        }
        nodes.push((
            *id,
            Passage {
                title: node.title.clone(),
                body: node.body.clone(),
                args: node.args.clone(),
                choice_points,
                next: node.next.as_deref().map(&resolve).transpose()?,
            },
        ));
    }
    Ok(Input::Propose {
        choice_point: request.choice_point,
        options,
        nodes,
    })
}

/// The canonical request a recorded proposal was made from. A reference to one of its own
/// nodes becomes that node's position.
fn request_of(
    choice_point: &ChoicePointId,
    options: &[OptionPlan],
    nodes: &[(NodeId, Passage)],
) -> ProposalRequest {
    let positions: BTreeMap<NodeId, usize> = nodes
        .iter()
        .enumerate()
        .map(|(index, (id, _))| (*id, index))
        .collect();
    let target = |node: &NodeId| {
        positions
            .get(node)
            .map_or_else(|| node.to_string(), usize::to_string)
    };
    let option = |option: &OptionPlan| OptionProposal {
        label: option.label.clone(),
        visible_if: option.visible_if.clone(),
        enabled_if: option.enabled_if.clone(),
        reason: option.reason.clone(),
        effects: option.effects.clone(),
        outcome: match &option.outcome {
            Outcome::Local { reply, rejoin } => OutcomeSource::Local {
                reply: reply.clone(),
                rejoin: rejoin.clone(),
            },
            Outcome::Branch { target: node } => OutcomeSource::Branch {
                target: target(node),
            },
        },
    };
    ProposalRequest {
        choice_point: *choice_point,
        options: options.iter().map(option).collect(),
        nodes: nodes
            .iter()
            .enumerate()
            .map(|(index, (_, passage))| NodeProposal {
                key: index.to_string(),
                title: passage.title.clone(),
                body: passage.body.clone(),
                args: passage.args.clone(),
                choice_points: passage
                    .choice_points
                    .iter()
                    .map(|point| ChoicePointProposal {
                        placement: point.placement.clone(),
                        min: point.min,
                        max: point.max,
                        proposals: point.proposals,
                        options: point.options.iter().map(option).collect(),
                    })
                    .collect(),
                next: passage.next.as_ref().map(target),
            })
            .collect(),
    }
}

/// Authored IDs in use.
#[derive(Default)]
struct Ids {
    nodes: BTreeSet<NodeId>,
    points: BTreeSet<ChoicePointId>,
    options: BTreeSet<OptionId>,
}

impl Ids {
    fn passage(&mut self, passage: &Passage) {
        for point in &passage.choice_points {
            self.points.insert(point.id);
            self.options
                .extend(point.options.iter().map(|option| option.id));
        }
    }

    fn graph(graph: &Graph) -> Self {
        let mut ids = Self {
            nodes: graph.nodes.keys().copied().collect(),
            ..Self::default()
        };
        for plan in graph.nodes.values() {
            if let Plan::Passage(passage) = plan {
                ids.passage(passage);
            }
        }
        ids
    }

    fn overlay(&mut self, overlay: &Overlay) {
        for (id, passage) in &overlay.nodes {
            self.nodes.insert(*id);
            self.passage(passage);
        }
        for options in overlay.options.values() {
            self.options.extend(options.iter().map(|option| option.id));
        }
    }
}

/// Derived IDs must be new to the frame's graph, the tombstone set and every overlay of the
/// state. Runtime addressing never leaves the frame's graph, so other chunks are not loaded
/// for this (ADR 0013 §3, §8); tool-minted IDs are UUIDv7 and cannot equal a derived one.
fn check_collisions(
    graph: &Graph,
    tombstones: &TombstoneSet,
    parent: &State,
    options: &[OptionPlan],
    nodes: &[(NodeId, Passage)],
) -> Result<()> {
    let mut taken = Ids::graph(graph);
    for frame in &parent.frames {
        taken.overlay(&frame.overlay);
    }
    let collision = |id: String| {
        Error::new(
            "collision",
            "proposal",
            format!("derived {id} is already in use or deleted"),
        )
    };
    let mut option = |option: &OptionPlan| {
        if taken.options.insert(option.id) && !tombstones.options.contains(&option.id) {
            Ok(())
        } else {
            Err(collision(option.id.to_string()))
        }
    };
    for value in options {
        option(value)?;
    }
    for (id, passage) in nodes {
        if !taken.nodes.insert(*id) || tombstones.nodes.contains(id) {
            return Err(collision(id.to_string()));
        }
        for point in &passage.choice_points {
            if !taken.points.insert(point.id) || tombstones.choice_points.contains(&point.id) {
                return Err(collision(point.id.to_string()));
            }
            for value in &point.options {
                option(value)?;
            }
        }
    }
    Ok(())
}

/// Checks a `propose` input against the state of its parent commit and returns the state it
/// leads to: the same interaction, with the proposal in the top frame's overlay.
pub(crate) fn apply_proposal(
    program: &Program,
    parent_commit: &CommitId,
    parent: &State,
    input: &Input,
) -> Result<State> {
    let Input::Propose {
        choice_point,
        options,
        nodes,
    } = input
    else {
        return Err(Error::new("state", "input", "expected a proposal"));
    };
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
    if !passage
        .choice_points
        .iter()
        .any(|point| point.id == *choice_point && point.proposals)
    {
        return Err(Error::new(
            "proposals",
            "proposal.choice_point",
            "the choice point does not accept proposals",
        ));
    }
    if frame.overlay.nodes.len() + nodes.len() > MAX_OVERLAY_NODES {
        return Err(Error::new(
            "limit",
            "proposal.nodes",
            "a frame holds at most 256 proposed nodes",
        ));
    }
    if materialize(parent_commit, &request_of(choice_point, options, nodes))? != *input {
        return Err(Error::new(
            "derived_id",
            "input",
            "the recorded IDs are not the ones derived from the parent commit and the request",
        ));
    }
    let tombstones = program.tombstones()?;
    check_collisions(&graph, tombstones, parent, options, nodes)?;
    let mut overlay = frame.overlay.clone();
    overlay
        .nodes
        .extend(nodes.iter().map(|(id, passage)| (*id, passage.clone())));
    if !options.is_empty() {
        overlay
            .options
            .entry(*choice_point)
            .or_default()
            .extend(options.iter().cloned());
    }
    let declarations = graph.header.declarations();
    let target = |node: &NodeId| -> Result<()> {
        if graph.nodes.contains_key(node) || overlay.nodes.contains_key(node) {
            Ok(())
        } else {
            Err(Error::new(
                "reference",
                "proposal",
                format!("target node {node} is neither in the graph nor proposed"),
            ))
        }
    };
    for (index, (_, passage)) in nodes.iter().enumerate() {
        check_passage(
            &declarations,
            passage,
            &target,
            tombstones,
            &mut BTreeSet::new(),
            &mut BTreeSet::new(),
            &format!("proposal.nodes[{index}]"),
        )?;
    }
    // The whole choice point after the append, and its passage's end rule.
    let composed = overlay
        .passage(&graph, &frame.node)
        .ok_or_else(|| Error::new("state", "input", "the current node is not a passage"))?;
    check_passage(
        &declarations,
        &composed,
        &target,
        tombstones,
        &mut BTreeSet::new(),
        &mut BTreeSet::new(),
        &node_path(None, &frame.graph, &frame.node),
    )?;
    let mut state = parent.clone();
    if let Some(top) = state.frames.last_mut() {
        top.overlay = overlay;
    }
    check_size(&state)?;
    Ok(state)
}

/// The overlay invariants of ADR 0013 §6 for one frame: proposed nodes and options are new,
/// not deleted, reference only the graph and the overlay, and every passage they change still
/// passes the compile-time rules.
pub(crate) fn check_overlay(program: &Program, graph: &Graph, frame: &Frame) -> Result<()> {
    let overlay = &frame.overlay;
    if overlay.is_empty() {
        return Ok(());
    }
    if overlay.nodes.len() > MAX_OVERLAY_NODES {
        return Err(Error::new(
            "limit",
            "overlay",
            "a frame holds at most 256 proposed nodes",
        ));
    }
    let tombstones = program.tombstones()?;
    let declarations = graph.header.declarations();
    let target = |node: &NodeId| -> Result<()> {
        if graph.nodes.contains_key(node) || overlay.nodes.contains_key(node) {
            Ok(())
        } else {
            Err(Error::new(
                "reference",
                "overlay",
                format!("target node {node} is neither in the graph nor proposed"),
            ))
        }
    };
    let Ids {
        mut points,
        mut options,
        ..
    } = Ids::graph(graph);
    let mut owners = BTreeMap::new();
    for (id, plan) in &graph.nodes {
        if let Plan::Passage(passage) = plan {
            owners.extend(passage.choice_points.iter().map(|point| (point.id, *id)));
        }
    }
    for (id, passage) in &overlay.nodes {
        let path = format!("overlay.nodes.{id}");
        if graph.nodes.contains_key(id) || tombstones.nodes.contains(id) {
            return Err(Error::new(
                "duplicate",
                &path,
                format!("{id} is a node of the graph or deleted"),
            ));
        }
        check_passage(
            &declarations,
            passage,
            &target,
            tombstones,
            &mut points,
            &mut options,
            &path,
        )?;
        owners.extend(passage.choice_points.iter().map(|point| (point.id, *id)));
    }
    let mut changed = BTreeSet::new();
    for (point, appended) in &overlay.options {
        let path = format!("overlay.options.{point}");
        let node = owners
            .get(point)
            .ok_or_else(|| Error::new("reference", &path, "options for an unknown choice point"))?;
        let accepts = overlay
            .passage(graph, node)
            .and_then(|passage| {
                passage
                    .choice_points
                    .iter()
                    .find(|candidate| candidate.id == *point)
                    .map(|candidate| candidate.proposals)
            })
            .unwrap_or(false);
        if !accepts || appended.is_empty() {
            return Err(Error::new(
                "proposals",
                &path,
                "options are only appended to choice points that accept proposals",
            ));
        }
        for option in appended {
            if !options.insert(option.id) || tombstones.options.contains(&option.id) {
                return Err(Error::new(
                    "duplicate",
                    &path,
                    format!("{} is already in use or deleted", option.id),
                ));
            }
        }
        changed.insert(*node);
    }
    for node in changed {
        let passage = overlay
            .passage(graph, &node)
            .ok_or_else(|| Error::new("state", "overlay", "missing passage"))?;
        check_passage(
            &declarations,
            &passage,
            &target,
            tombstones,
            &mut BTreeSet::new(),
            &mut BTreeSet::new(),
            &node_path(None, &frame.graph, &node),
        )?;
    }
    Ok(())
}
