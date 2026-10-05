//! Ephemeral observations of the same machine used by sessions. No trace has a wire form.

use super::{Item, Machine, Step};
use crate::{
    ChoicePointId, CommitId, Frame, Input, NodeId, OptionId, Program, Result, State,
    plan::{GraphRef, NameTable, Outcome},
    state::{check_size, check_state},
    view::{Interaction, PresentationItem, Role},
};
use narrata_kernel::content::Segment;

/// The graph instance that executed an event. Repeated calls remain distinguishable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceLocation {
    pub graph: GraphRef,
    pub node: NodeId,
    pub instance: u32,
}

impl From<&Frame> for TraceLocation {
    fn from(frame: &Frame) -> Self {
        Self {
            graph: frame.graph.clone(),
            node: frame.node,
            instance: frame.instance,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum TraceAction {
    Visit,
    Segment {
        role: Role,
        segment: Segment,
    },
    Selected {
        choice_point: ChoicePointId,
        option: OptionId,
        outcome: Outcome,
    },
    Branch {
        target: NodeId,
    },
    Call {
        target: TraceLocation,
    },
    Return {
        outcome: String,
        continuation: Option<TraceLocation>,
    },
    Finished {
        outcome: String,
        body: Option<Segment>,
    },
}

/// One executed observation, in execution order. Accessors distinguish actions without
/// exposing a second execution vocabulary or an exchanged representation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceEvent {
    pub location: TraceLocation,
    pub(super) action: TraceAction,
}

impl TraceEvent {
    /// True only when entering a node, including automatic control nodes. Resuming a
    /// passage after a local choice does not enter it again.
    pub fn is_visit(&self) -> bool {
        matches!(self.action, TraceAction::Visit)
    }

    /// Every presented body/reply segment, including the ending's interaction body.
    pub fn segment(&self) -> Option<&Segment> {
        match &self.action {
            TraceAction::Segment { segment, .. } => Some(segment),
            TraceAction::Finished { body, .. } => body.as_ref(),
            _ => None,
        }
    }

    /// Ending bodies have no presentation role: they belong to `Interaction::Finished`.
    pub fn role(&self) -> Option<Role> {
        match self.action {
            TraceAction::Segment { role, .. } => Some(role),
            _ => None,
        }
    }

    /// The checked outcome, before its effects run. For control-path comparison, compare
    /// the outcome kind and branch target; local reply references belong to presentation.
    pub fn selection(&self) -> Option<(ChoicePointId, OptionId, &Outcome)> {
        match &self.action {
            TraceAction::Selected {
                choice_point,
                option,
                outcome,
            } => Some((*choice_point, *option, outcome)),
            _ => None,
        }
    }

    pub fn branch_target(&self) -> Option<NodeId> {
        match self.action {
            TraceAction::Branch { target } => Some(target),
            _ => None,
        }
    }

    pub fn call_target(&self) -> Option<&TraceLocation> {
        match &self.action {
            TraceAction::Call { target } => Some(target),
            _ => None,
        }
    }

    /// A root return has no continuation.
    pub fn returned(&self) -> Option<(&str, Option<&TraceLocation>)> {
        match &self.action {
            TraceAction::Return {
                outcome,
                continuation,
            } => Some((outcome, continuation.as_ref())),
            _ => None,
        }
    }

    pub fn ending(&self) -> Option<(&str, Option<&Segment>)> {
        match &self.action {
            TraceAction::Finished { outcome, body } => Some((outcome, body.as_ref())),
            _ => None,
        }
    }
}

/// One successful initial/advance observation. Failed execution returns no partial trace.
/// It owns its events and can be dropped after each replayed input. It never writes history.
pub struct TracedStep {
    pub state: State,
    pub events: Vec<TraceEvent>,
    pub entered: bool,
    items: Vec<Item>,
}

impl From<Step> for TracedStep {
    fn from(step: Step) -> Self {
        Self {
            state: step.state,
            events: step.trace,
            entered: step.entered,
            items: step.presentation,
        }
    }
}

impl TracedStep {
    /// The ordinary interaction from the observed result state.
    pub fn interaction(&self, program: &Program, names: Option<&NameTable>) -> Result<Interaction> {
        Machine { program }.interaction(&self.state, names)
    }

    /// The ordinary presentation, with the host's resulting commit identity attached.
    /// Ending bodies remain in the interaction and trace, not in this presentation.
    pub fn presentation(&self, commit: CommitId) -> Vec<PresentationItem> {
        self.items
            .iter()
            .enumerate()
            .map(|(occurrence, item)| PresentationItem {
                commit,
                occurrence: occurrence as u32,
                role: item.role,
                node: item.node,
                content: item.content.clone(),
                args: item.args.clone(),
            })
            .collect()
    }
}

impl Program {
    /// Runs the manifest's initial state through the ordinary runtime, observing every
    /// node entry, segment and control transition up to its first interaction or ending.
    /// This entry point exists only with the `trace` feature.
    pub fn trace_initial(&self) -> Result<TracedStep> {
        Machine { program: self }.initial().map(Into::into)
    }

    /// Read-only shadow execution of one recorded input. `parent_commit` is the original
    /// input parent identity (including an interim identity when replaying an old proposal).
    /// Typed states are mutable outside the runtime, so validate the parent before use.
    /// Proposals are checked by the ordinary proposal path; they produce no execution
    /// events until a proposed option is selected or a proposed node is entered.
    pub fn trace_apply(
        &self,
        parent_commit: &CommitId,
        parent: &State,
        input: &Input,
    ) -> Result<TracedStep> {
        check_size(parent)?;
        check_state(self, parent)?;
        Machine { program: self }
            .apply(parent_commit, parent, input)
            .map(Into::into)
    }
}
