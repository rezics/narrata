//! The read model exchanged with hosts (ADR 0013 §7). It carries references and values, never
//! text; aliases appear only when a name table is loaded. It is not part of any digest.

use std::collections::{BTreeMap, BTreeSet};

use narrata_kernel::content::{ContentRef, Segment};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ArtifactId, ChoicePointId, CommitId, Diagnostic, ExecutionId, NodeId, OptionId, ViewScalar,
    analysis::GraphAnalysis, plan::GraphRef,
};

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionView {
    pub artifact_id: ArtifactId,
    pub execution: ExecutionId,
    pub cursor: CommitId,
    pub depth: u64,
    pub product: ProductView,
    /// What to show since the previous interaction, recomputed from the parent state and the
    /// input rather than stored.
    pub presentation: Vec<PresentationItem>,
    pub interaction: Interaction,
    pub shared: Vec<VariableView>,
    pub frames: Vec<FrameView>,
    pub history: Vec<HistoryView>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductView {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<ContentRef>,
}

/// One item to present. `(execution, commit, occurrence)` is the presentation key that
/// generative content providers cache by.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PresentationItem {
    /// The commit whose step presented the item.
    pub commit: CommitId,
    /// The item's position in that commit's presentation.
    pub occurrence: u32,
    pub role: Role,
    pub node: NodeId,
    pub content: Presented,
    /// The passage's named arguments when the item was presented.
    pub args: BTreeMap<String, ViewScalar>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Title,
    Body,
    Reply,
}

/// A title is a reference; body and reply items are segments.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Presented {
    Segment(Segment),
    Ref(ContentRef),
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Interaction {
    Choose {
        graph: GraphRef,
        node: NodeId,
        choice_point: ChoicePointId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        min: u16,
        max: u16,
        /// Whether the host may propose options and passages here.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        proposals: bool,
        /// Arguments for option labels and disabled reasons.
        args: BTreeMap<String, ViewScalar>,
        /// Visible options in order, proposed ones after the choice point's own; hidden ones
        /// are left out.
        options: Vec<OptionView>,
    },
    Finished {
        outcome: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<ContentRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<Segment>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionView {
    pub id: OptionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<ContentRef>,
    pub enabled: bool,
    /// Only for a disabled option that declares a reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<ContentRef>,
    pub outcome: OutcomeKind,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    Local,
    Branch,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VariableView {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<ContentRef>,
    pub value: ViewScalar,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrameView {
    pub graph: GraphRef,
    pub node: NodeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<ChoicePointId>,
    pub instance: u32,
    pub parameters: Vec<VariableView>,
    pub locals: Vec<VariableView>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryView {
    pub id: CommitId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<CommitId>,
    pub depth: u64,
    /// Where the commit waits, or the node that finished the story.
    pub graph: GraphRef,
    pub node: NodeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub instance: u32,
    /// The waiting passage's title, or the ending's title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<ContentRef>,
    pub finished: bool,
    pub current: bool,
}

/// What the browser reader receives: the session view, the reading page of the current
/// passage, and the text-free graph analysis.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BookView {
    pub view: SessionView,
    /// The presentation of the step that entered the current passage (or ended the story),
    /// followed by those of the local choices made in it since.
    pub page: Vec<PresentationItem>,
    pub graphs: Vec<GraphAnalysis>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Sorted, unique body units that may be presented after one choice, up to the next
/// interaction. Current hidden/disabled options are excluded. Future conditions are
/// conservatively followed both ways: effects and multi-selection can change them.
/// Only references escape; no future node IDs, labels, arguments or topology do.
/// This reads checked program chunks and never advances the session or resolves content.
pub fn next_content_units(
    program: &crate::Program,
    state: &crate::State,
) -> crate::Result<Vec<ContentRef>> {
    use crate::{
        Error, MAX_CALL_DEPTH, MAX_STEPS,
        plan::{Outcome, Plan},
        runtime::Machine,
    };

    let Interaction::Choose { options, .. } = (Machine { program }).interaction(state, None)?
    else {
        return Ok(Vec::new());
    };
    let frame = state
        .frames
        .last()
        .ok_or_else(|| Error::new("state", "prefetch", "no active frame"))?;
    let graph = program.graph(&frame.graph)?;
    let passage = frame
        .overlay
        .passage(&graph, &frame.node)
        .ok_or_else(|| Error::new("state", "prefetch", "not a passage"))?;
    let (index, point) = passage
        .choice_points
        .iter()
        .enumerate()
        .find(|(_, point)| Some(point.id) == frame.at)
        .ok_or_else(|| Error::new("state", "prefetch", "missing choice point"))?;
    let enabled: BTreeSet<_> = options
        .iter()
        .filter(|option| option.enabled)
        .map(|option| option.id)
        .collect();
    let mut units = BTreeSet::new();
    let mut pending = Vec::new();
    let mut local = point.min == 0;
    let mut rejoin = point.min == 0
        && point.options.first().is_some_and(|option| {
            matches!(
                &option.outcome,
                Outcome::Local {
                    rejoin: Some(_),
                    ..
                }
            )
        });
    for option in point
        .options
        .iter()
        .filter(|option| enabled.contains(&option.id))
    {
        match &option.outcome {
            Outcome::Local {
                reply,
                rejoin: anchor,
            } => {
                local = true;
                rejoin |= anchor.is_some();
                units.extend(reply.iter().map(|segment| segment.unit.clone()));
            }
            Outcome::Branch { target } => {
                let mut frames = state.frames.clone();
                if let Some(top) = frames.last_mut() {
                    top.node = *target;
                    top.at = None;
                }
                pending.push(frames);
            }
        }
    }
    if local {
        rejoin |= passage.choice_points[index + 1..]
            .iter()
            .take_while(|point| point.min == 0 && !point.proposals)
            .any(|point| {
                point.options.first().is_some_and(|option| {
                    matches!(
                        &option.outcome,
                        Outcome::Local {
                            rejoin: Some(_),
                            ..
                        }
                    )
                })
            });
        if rejoin {
            units.extend(passage.body.iter().map(|segment| segment.unit.clone()));
        }
        // An optional point can disappear after effects when it has no enabled options.
        if passage.choice_points[index + 1..]
            .iter()
            .all(|point| point.min == 0 && !point.proposals)
            && let Some(next) = passage.next
        {
            let mut frames = state.frames.clone();
            if let Some(top) = frames.last_mut() {
                top.node = next;
                top.at = None;
            }
            pending.push(frames);
        }
    }
    let mut seen = BTreeSet::new();
    while let Some(mut frames) = pending.pop() {
        // Include callers in the key: one callee can return into different passages.
        let key: Vec<_> = frames
            .iter()
            .map(|frame| (frame.graph.clone(), frame.node, frame.instance))
            .collect();
        if !seen.insert(key) {
            continue;
        }
        if seen.len() > MAX_STEPS as usize {
            return Err(Error::new(
                "limit",
                "prefetch",
                "content lookahead exceeds 4096 locations",
            ));
        }
        let frame = frames
            .last()
            .ok_or_else(|| Error::new("state", "prefetch", "no active frame"))?;
        let graph = program.graph(&frame.graph)?;
        let plan = frame
            .overlay
            .plan(&graph, &frame.node)
            .ok_or_else(|| Error::new("state", "prefetch", "unknown node"))?;
        let next = match &*plan {
            Plan::Passage(passage) => {
                units.extend(passage.body.iter().map(|segment| segment.unit.clone()));
                if passage
                    .choice_points
                    .iter()
                    .all(|point| point.min == 0 && !point.proposals)
                {
                    passage.next
                } else {
                    None
                }
            }
            Plan::Mutate { next, .. } => Some(*next),
            Plan::Branch {
                when_true,
                when_false,
                ..
            } => {
                let mut alternative = frames.clone();
                if let Some(top) = alternative.last_mut() {
                    top.node = *when_false;
                }
                pending.push(alternative);
                Some(*when_true)
            }
            Plan::Call { target, .. } => {
                // A deeper call cannot successfully execute at the runtime's depth limit.
                if frames.len() < MAX_CALL_DEPTH {
                    let callee = program.resolve_call(&frame.graph, target)?;
                    let graph = program.graph(&callee)?;
                    frames.push(crate::Frame {
                        graph: callee,
                        node: graph.header.entry,
                        at: None,
                        instance: 0,
                        parameters: BTreeMap::new(),
                        locals: BTreeMap::new(),
                        overlay: crate::Overlay::default(),
                    });
                    pending.push(frames);
                }
                continue;
            }
            Plan::Return { outcome } => {
                let outcome = outcome.clone();
                frames.pop();
                if let Some(caller) = frames.last() {
                    let graph = program.graph(&caller.graph)?;
                    let plan = caller
                        .overlay
                        .plan(&graph, &caller.node)
                        .ok_or_else(|| Error::new("state", "prefetch", "unknown caller"))?;
                    let Plan::Call { on_return, .. } = &*plan else {
                        return Err(Error::new("state", "prefetch", "parent is not a call"));
                    };
                    on_return.get(&outcome).copied()
                } else {
                    units.extend(
                        program
                            .manifest()
                            .product
                            .endings
                            .get(&outcome)
                            .and_then(|ending| ending.body.as_ref())
                            .map(|segment| segment.unit.clone()),
                    );
                    None
                }
            }
        };
        if let Some(next) = next {
            if let Some(top) = frames.last_mut() {
                top.node = next;
                top.at = None;
            }
            pending.push(frames);
        }
    }
    Ok(units.into_iter().collect())
}
