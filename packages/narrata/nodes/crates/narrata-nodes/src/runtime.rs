//! Deterministic execution (ADR 0013 §4, §7). One input runs in a working copy of the parent
//! state until the next interaction or the end of the story; any failure discards the copy.
//! The runtime only emits references; it never reads content.

use std::collections::BTreeMap;

use narrata_kernel::content::{AnchorId, Segment};

use crate::{
    CommitId, Error, MAX_CALL_DEPTH, MAX_STEPS, NodeId, Program, Result, Scalar, Scope, ViewScalar,
    expr::{Values, expect},
    plan::{GraphRef, NameTable, Outcome, Passage, Plan},
    proposal::apply_proposal,
    state::{Finished, Frame, Input, Overlay, State, check_choice, check_size},
    view::{Interaction, OptionView, OutcomeKind, Presented, Role},
};

/// A presentation item before it is tied to the commit that the step becomes.
#[derive(Clone, Debug)]
pub(crate) struct Item {
    pub role: Role,
    pub node: NodeId,
    pub content: Presented,
    pub args: BTreeMap<String, ViewScalar>,
}

/// The result of running one input (or of starting the story).
pub(crate) struct Step {
    pub state: State,
    pub pins: crate::ChunkPins,
    pub presentation: Vec<Item>,
    /// Whether the passage the step stops in was entered during this step, rather than
    /// continued after a local choice.
    pub entered: bool,
}

struct Work {
    state: State,
    pins: crate::ChunkPins,
    items: Vec<Item>,
    steps: u32,
    entered: bool,
}

impl Work {
    fn top(&self) -> Result<&Frame> {
        self.state
            .frames
            .last()
            .ok_or_else(|| Error::new("finished", "session", "no active graph instance"))
    }

    fn top_mut(&mut self) -> Result<&mut Frame> {
        self.state
            .frames
            .last_mut()
            .ok_or_else(|| Error::new("finished", "session", "no active graph instance"))
    }

    fn values(&self) -> Result<Values<'_>> {
        let frame = self.top()?;
        Ok(Values {
            parameters: &frame.parameters,
            locals: &frame.locals,
            shared: &self.state.shared,
        })
    }

    fn tick(&mut self) -> Result<()> {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            return Err(Error::new(
                "step_limit",
                "execution",
                "automatic node transitions exceeded 4096 steps without an interaction",
            ));
        }
        Ok(())
    }

    fn present(
        &mut self,
        role: Role,
        node: NodeId,
        content: Presented,
        args: &BTreeMap<String, ViewScalar>,
    ) {
        self.items.push(Item {
            role,
            node,
            content,
            args: args.clone(),
        });
    }

    fn assign(&mut self, assignments: &[crate::Assignment]) -> Result<()> {
        for assignment in assignments {
            let value = self.values()?.evaluate(&assignment.value)?;
            let target = match assignment.target.scope {
                Scope::Local => self.top_mut()?.locals.get_mut(&assignment.target.name),
                Scope::Shared => self.state.shared.get_mut(&assignment.target.name),
                Scope::Parameter => {
                    return Err(Error::new(
                        "readonly",
                        "assignment",
                        "parameters cannot be modified",
                    ));
                }
            }
            .ok_or_else(|| {
                Error::new("state", "assignment", "missing checked assignment target")
            })?;
            expect(value.kind(), target.kind(), "assignment")?;
            *target = value;
        }
        Ok(())
    }

    fn args(&self, passage: &Passage) -> Result<BTreeMap<String, ViewScalar>> {
        let values = self.values()?;
        passage
            .args
            .iter()
            .map(|(name, value)| Ok((name.clone(), values.evaluate(value)?.view())))
            .collect()
    }
}

pub(crate) struct Machine<'p> {
    pub program: &'p Program,
}

impl Machine<'_> {
    /// The state after the manifest's initial state runs to its first interaction.
    pub fn initial(&self) -> Result<Step> {
        let product = &self.program.manifest().product;
        let graph = self.program.graph(&product.entry)?;
        let state = State {
            shared: product
                .shared
                .iter()
                .map(|(name, variable)| (name.clone(), variable.value.clone()))
                .collect(),
            frames: vec![Frame {
                graph: product.entry.clone(),
                node: graph.header.entry,
                at: None,
                instance: 1,
                parameters: product.arguments.clone(),
                locals: graph.header.locals.clone(),
                overlay: Overlay::default(),
            }],
            next_instance: 2,
            finished: None,
        };
        let mut work = Work {
            pins: self.program.pin_state(&state)?,
            state,
            items: Vec::new(),
            steps: 0,
            entered: false,
        };
        self.run(&mut work)?;
        check_size(&work.state)?;
        Ok(Step {
            state: work.state,
            pins: work.pins,
            presentation: work.items,
            entered: work.entered,
        })
    }

    /// Applies an input to the state of commit `parent_commit`.
    pub fn apply(&self, parent_commit: &CommitId, parent: &State, input: &Input) -> Result<Step> {
        match input {
            Input::Choose {
                choice_point,
                options,
            } => self.choose(parent, choice_point, options),
            Input::Propose { .. } => {
                let state = apply_proposal(self.program, parent_commit, parent, input)?;
                Ok(Step {
                    pins: self.program.pin_state(&state)?,
                    state,
                    presentation: Vec::new(),
                    entered: false,
                })
            }
        }
    }

    /// Chooses options at the parent's interaction. Every chosen option is checked against
    /// the parent state, then all effects run in option order, then replies are presented
    /// with arguments evaluated after the effects.
    fn choose(
        &self,
        parent: &State,
        choice_point: &crate::ChoicePointId,
        options: &[crate::OptionId],
    ) -> Result<Step> {
        let indices = check_choice(self.program, parent, choice_point, options)?;
        let mut work = Work {
            pins: self.program.pin_state(parent)?,
            state: parent.clone(),
            items: Vec::new(),
            steps: 0,
            entered: false,
        };
        work.tick()?;
        let (graph_ref, node, at) = {
            let frame = work.top()?;
            (frame.graph.clone(), frame.node, frame.at)
        };
        let graph = self.program.graph(&graph_ref)?;
        let passage = work
            .top()?
            .overlay
            .passage(&graph, &node)
            .ok_or_else(|| Error::new("state", "input", "the current node is not a passage"))?;
        let (index, point) = passage
            .choice_points
            .iter()
            .enumerate()
            .find(|(_, point)| Some(point.id) == at)
            .ok_or_else(|| Error::new("state", "input", "missing current choice point"))?;
        let chosen: Vec<_> = indices
            .iter()
            .filter_map(|index| point.options.get(*index))
            .collect();
        {
            let values = work.values()?;
            for option in &chosen {
                if !values.condition(option.visible_if.as_ref())?
                    || !values.condition(option.enabled_if.as_ref())?
                {
                    return Err(Error::new(
                        "unavailable",
                        "input",
                        "option conditions are not satisfied",
                    ));
                }
            }
        }
        for option in &chosen {
            work.assign(&option.effects)?;
        }
        work.top_mut()?.at = None;
        match chosen.as_slice() {
            [option] if matches!(option.outcome, Outcome::Branch { .. }) => {
                if let Outcome::Branch { target } = &option.outcome {
                    work.top_mut()?.node = *target;
                }
            }
            _ => {
                let args = work.args(&passage)?;
                for option in &chosen {
                    if let Outcome::Local {
                        reply: Some(reply), ..
                    } = &option.outcome
                    {
                        work.present(Role::Reply, node, Presented::Segment(reply.clone()), &args);
                    }
                }
                // Multiple selection and min = 0 share one rejoin, so the first option's (or,
                // for an empty selection, the choice point's) rejoin applies to all.
                let rejoin = chosen
                    .first()
                    .copied()
                    .or_else(|| point.options.first())
                    .and_then(|option| match &option.outcome {
                        Outcome::Local { rejoin, .. } => rejoin.clone(),
                        Outcome::Branch { .. } => None,
                    });
                self.after_local(&mut work, node, &passage, index, rejoin)?;
            }
        }
        self.run(&mut work)?;
        check_size(&work.state)?;
        Ok(Step {
            state: work.state,
            pins: work.pins,
            presentation: work.items,
            entered: work.entered,
        })
    }

    fn run(&self, work: &mut Work) -> Result<()> {
        loop {
            if work.state.finished.is_some() {
                return Ok(());
            }
            // The overlay stays in place: copying it on every step would cost its size.
            let (graph_ref, node, instance) = {
                let frame = work.top()?;
                if frame.at.is_some() {
                    return Ok(());
                }
                (frame.graph.clone(), frame.node, frame.instance)
            };
            work.tick()?;
            let graph = self.program.graph(&graph_ref)?;
            let plan = work
                .top()?
                .overlay
                .plan(&graph, &node)
                .ok_or_else(|| Error::new("state", node.to_string(), "unknown checked node"))?;
            match &*plan {
                Plan::Passage(passage) => self.enter(work, node, passage)?,
                Plan::Branch {
                    condition,
                    when_true,
                    when_false,
                } => {
                    let next = if work.values()?.boolean(condition)? {
                        when_true
                    } else {
                        when_false
                    };
                    work.top_mut()?.node = *next;
                }
                Plan::Mutate { assignments, next } => {
                    work.assign(assignments)?;
                    work.top_mut()?.node = *next;
                    check_size(&work.state)?;
                }
                Plan::Call {
                    target, arguments, ..
                } => {
                    if work.state.frames.len() >= MAX_CALL_DEPTH {
                        return Err(Error::new(
                            "depth_limit",
                            graph_ref.label(),
                            "subgraph call depth exceeds 64",
                        ));
                    }
                    let callee_ref = self.program.resolve_call(&graph_ref, target)?;
                    let callee = self.program.graph(&callee_ref)?;
                    let parameters = {
                        let values = work.values()?;
                        arguments
                            .iter()
                            .map(|(name, value)| Ok((name.clone(), values.evaluate(value)?)))
                            .collect::<Result<BTreeMap<String, Scalar>>>()?
                    };
                    let instance = work.state.next_instance;
                    work.state.next_instance = instance.checked_add(1).ok_or_else(|| {
                        Error::new("limit", "instances", "instance counter overflow")
                    })?;
                    work.state.frames.push(Frame {
                        graph: callee_ref,
                        node: callee.header.entry,
                        at: None,
                        instance,
                        parameters,
                        locals: callee.header.locals.clone(),
                        overlay: Overlay::default(),
                    });
                    work.pins = self.program.pin_state(&work.state)?;
                    check_size(&work.state)?;
                }
                Plan::Return { outcome } => {
                    work.state.frames.pop();
                    work.pins = self.program.pin_state(&work.state)?;
                    match work.state.frames.last_mut() {
                        Some(parent) => {
                            let caller = self.program.graph(&parent.graph)?;
                            let Some(Plan::Call { on_return, .. }) = caller.nodes.get(&parent.node)
                            else {
                                return Err(Error::new(
                                    "state",
                                    "return",
                                    "parent is not a call site",
                                ));
                            };
                            parent.node = *on_return.get(outcome).ok_or_else(|| {
                                Error::new("state", "return", "missing outcome continuation")
                            })?;
                        }
                        None => {
                            work.state.finished = Some(Finished {
                                node,
                                instance,
                                outcome: outcome.clone(),
                            });
                        }
                    }
                }
            }
        }
    }

    /// Presents the title and the first stretch of body, then arrives at the first choice
    /// point (or runs to the end of the passage).
    fn enter(&self, work: &mut Work, node: NodeId, passage: &Passage) -> Result<()> {
        work.entered = true;
        let args = work.args(passage)?;
        if let Some(title) = &passage.title {
            work.present(Role::Title, node, Presented::Ref(title.clone()), &args);
        }
        if let Some(body) = &passage.body {
            let last = match passage.choice_points.first() {
                Some(point) => point.placement.clone().or_else(|| body.last.clone()),
                None => body.last.clone(),
            };
            work.present(
                Role::Body,
                node,
                Presented::Segment(Segment {
                    unit: body.unit.clone(),
                    first: body.first.clone(),
                    last,
                }),
                &args,
            );
        }
        self.arrive(work, node, passage, 0)
    }

    /// Presents the stretch from `rejoin` to the next choice point (or the body's end), then
    /// arrives at the next choice point.
    fn after_local(
        &self,
        work: &mut Work,
        node: NodeId,
        passage: &Passage,
        index: usize,
        rejoin: Option<AnchorId>,
    ) -> Result<()> {
        if let (Some(rejoin), Some(body)) = (rejoin, &passage.body) {
            let args = work.args(passage)?;
            let last = match passage.choice_points.get(index + 1) {
                Some(point) => point.placement.clone().or_else(|| body.last.clone()),
                None => body.last.clone(),
            };
            work.present(
                Role::Body,
                node,
                Presented::Segment(Segment {
                    unit: body.unit.clone(),
                    first: Some(rejoin),
                    last,
                }),
                &args,
            );
        }
        self.arrive(work, node, passage, index + 1)
    }

    fn arrive(&self, work: &mut Work, node: NodeId, passage: &Passage, index: usize) -> Result<()> {
        let Some(point) = passage.choice_points.get(index) else {
            let next = passage.next.ok_or_else(|| {
                Error::new(
                    "state",
                    node.to_string(),
                    "the passage ended without a next node",
                )
            })?;
            let frame = work.top_mut()?;
            frame.node = next;
            frame.at = None;
            return Ok(());
        };
        if point.proposals {
            // Proposals can add options, so the interaction appears even when no option is
            // available (ADR 0013 §5).
            work.top_mut()?.at = Some(point.id);
            return Ok(());
        }
        let available = {
            let values = work.values()?;
            let mut count = 0_usize;
            for option in &point.options {
                if values.condition(option.visible_if.as_ref())?
                    && values.condition(option.enabled_if.as_ref())?
                {
                    count += 1;
                }
            }
            count
        };
        if point.min == 0 && available == 0 {
            // Choosing nothing is the only possibility: skip without an interaction.
            let rejoin = point
                .options
                .first()
                .and_then(|option| match &option.outcome {
                    Outcome::Local { rejoin, .. } => rejoin.clone(),
                    Outcome::Branch { .. } => None,
                });
            return self.after_local(work, node, passage, index, rejoin);
        }
        if available < usize::from(point.min) {
            return Err(Error::new(
                "no_actions",
                node.to_string(),
                "fewer options are available than the choice point requires",
            ));
        }
        work.top_mut()?.at = Some(point.id);
        Ok(())
    }

    /// The interaction a persisted state waits at.
    pub fn interaction(&self, state: &State, names: Option<&NameTable>) -> Result<Interaction> {
        if let Some(finished) = &state.finished {
            let ending = self
                .program
                .manifest()
                .product
                .endings
                .get(&finished.outcome)
                .cloned()
                .unwrap_or_default();
            return Ok(Interaction::Finished {
                outcome: finished.outcome.clone(),
                title: ending.title,
                body: ending.body,
            });
        }
        let frame = state
            .frames
            .last()
            .ok_or_else(|| Error::new("state", "session", "no active graph instance"))?;
        let graph = self.program.graph(&frame.graph)?;
        let passage = frame
            .overlay
            .passage(&graph, &frame.node)
            .ok_or_else(|| Error::new("state", "session", "the current node is not a passage"))?;
        let point = passage
            .choice_points
            .iter()
            .find(|point| Some(point.id) == frame.at)
            .ok_or_else(|| Error::new("state", "session", "missing current choice point"))?;
        let values = Values {
            parameters: &frame.parameters,
            locals: &frame.locals,
            shared: &state.shared,
        };
        let args = passage
            .args
            .iter()
            .map(|(name, value)| Ok((name.clone(), values.evaluate(value)?.view())))
            .collect::<Result<_>>()?;
        let mut options = Vec::new();
        for option in &point.options {
            if !values.condition(option.visible_if.as_ref())? {
                continue;
            }
            let enabled = values.condition(option.enabled_if.as_ref())?;
            options.push(OptionView {
                id: option.id,
                key: names
                    .and_then(|names| names.option(&frame.graph, &option.id).map(str::to_owned)),
                label: option.label.clone(),
                enabled,
                reason: if enabled { None } else { option.reason.clone() },
                outcome: match option.outcome {
                    Outcome::Local { .. } => OutcomeKind::Local,
                    Outcome::Branch { .. } => OutcomeKind::Branch,
                },
            });
        }
        Ok(Interaction::Choose {
            graph: frame.graph.clone(),
            node: frame.node,
            choice_point: point.id,
            key: names.and_then(|names| {
                names
                    .choice_point(&frame.graph, &point.id)
                    .map(str::to_owned)
            }),
            min: point.min,
            max: point.max,
            proposals: point.proposals,
            args,
            options,
        })
    }

    /// The title shown for a commit in history: the waiting passage's or the ending's.
    pub fn title(
        &self,
        state: &State,
    ) -> Result<(
        GraphRef,
        NodeId,
        u32,
        Option<narrata_kernel::content::ContentRef>,
    )> {
        let product = &self.program.manifest().product;
        if let Some(finished) = &state.finished {
            let title = product
                .endings
                .get(&finished.outcome)
                .and_then(|ending| ending.title.clone());
            return Ok((
                product.entry.clone(),
                finished.node,
                finished.instance,
                title,
            ));
        }
        let frame = state
            .frames
            .last()
            .ok_or_else(|| Error::new("state", "session", "no active graph instance"))?;
        let graph = self.program.graph(&frame.graph)?;
        let title = frame
            .overlay
            .passage(&graph, &frame.node)
            .and_then(|passage| passage.title.clone());
        Ok((frame.graph.clone(), frame.node, frame.instance, title))
    }
}
