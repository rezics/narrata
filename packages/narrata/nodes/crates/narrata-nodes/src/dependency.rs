//! Bounded, read-only projection of what a session may need after its current safe point.
//!
//! The projection is an in-memory query for later preparation work, not a prediction. It lists
//! the logical graph objects, content references and media references that *some* continuation
//! from the current interaction could present at the next safe point, without evaluating the
//! story: branch conditions, assignments, call arguments and the visibility of future choices
//! are not computed, so the answer is a conservative union that may name references the reader
//! never reaches. It never advances the session, never mutates state, refs or history, and loads
//! program chunks only through the checked [`Program`] cache.
//!
//! # Scope of the walk
//!
//! * At a dialogue the walk starts at its `next` node. At a choice it starts from every option
//!   that is visible and enabled in the current state (hidden and disabled options are left
//!   out), plus the "nothing chosen" path when `min` is zero. Multi-select combinations are
//!   never enumerated: the targets of all enabled options are united.
//! * Automatic nodes (`Scene`, `Mutate`, `Branch`, `Call`, `Return`) are followed with the
//!   checked semantics of the runtime: both sides of a branch, the callee and then the matching
//!   `on_return` continuation of the tracked call stack, and the active callers after the
//!   outermost return.
//! * Every path stops at the first node where the reader would act again (dialogue, or a passage
//!   whose choice point is certain to appear) or at an ending. Nothing past that safe point is
//!   reported.
//!
//! # Bounds and completeness
//!
//! [`DependencyLimits`] bounds the visited locations, the graphs loaded beyond those already
//! held by the current frames, the candidate references considered and the distinct references
//! returned. The walk is breadth-first so the nearest candidates survive a small bound, and it
//! stops *before* work beyond a bound. Whatever stopped it, or whatever could not be loaded,
//! is listed in [`DependencyProjection::truncations`]; the result is complete only when that
//! list is empty. A load failure keeps the source's own [`Error`] and the exact chunk object
//! identity, and says only that the object was unavailable, never that it does not exist. The
//! set of things that were not evaluated is reported separately in
//! [`DependencyProjection::unknowns`]; it describes how conservative a complete result is.

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};

use narrata_kernel::content::ContentRef;

use crate::{
    ArtifactId, CommitId, Error, MAX_CALL_DEPTH, MAX_STEPS, NodeId, ObjectId, OptionId, Program,
    Result, State,
    plan::{ChoicePoint, Graph, GraphRef, MediaRef, MusicAction, Outcome, Passage, Plan},
    runtime::Machine,
    view::Interaction,
};

/// Checked bounds of one projection. Every limit is at least one, except `graphs`, where zero
/// restricts the walk to the graphs that the current frames already hold.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DependencyLimits {
    locations: u32,
    graphs: u32,
    candidates: u32,
    references: u32,
}

impl DependencyLimits {
    /// The most locations a projection may visit: the runtime's automatic step bound.
    pub const MAX_LOCATIONS: u32 = MAX_STEPS;
    pub const MAX_GRAPHS: u32 = 64;
    pub const MAX_CANDIDATES: u32 = 16 * 1024;
    pub const MAX_REFERENCES: u32 = 4 * 1024;

    /// * `locations`: distinct (call stack, node, music) places visited by the walk.
    /// * `graphs`: graphs loaded beyond those the current frames already hold.
    /// * `candidates`: references considered, counting repeats.
    /// * `references`: distinct graph, content and media references returned.
    pub fn new(locations: u32, graphs: u32, candidates: u32, references: u32) -> Result<Self> {
        let check = |name: &str, value: u32, min: u32, max: u32| {
            if (min..=max).contains(&value) {
                Ok(())
            } else {
                Err(Error::new(
                    "limit",
                    format!("dependency_limits.{name}"),
                    format!("{name} must be between {min} and {max}"),
                ))
            }
        };
        check("locations", locations, 1, Self::MAX_LOCATIONS)?;
        check("graphs", graphs, 0, Self::MAX_GRAPHS)?;
        check("candidates", candidates, 1, Self::MAX_CANDIDATES)?;
        check("references", references, 1, Self::MAX_REFERENCES)?;
        Ok(Self {
            locations,
            graphs,
            candidates,
            references,
        })
    }

    pub fn locations(&self) -> u32 {
        self.locations
    }

    pub fn graphs(&self) -> u32 {
        self.graphs
    }

    pub fn candidates(&self) -> u32 {
        self.candidates
    }

    pub fn references(&self) -> u32 {
        self.references
    }
}

impl Default for DependencyLimits {
    fn default() -> Self {
        Self {
            locations: 256,
            graphs: 8,
            candidates: 2048,
            references: 512,
        }
    }
}

/// What the references are for. A host decides from the role how urgently to prepare one.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ContentRole {
    DialogueLine,
    DialogueSpeaker,
    /// A passage's title.
    PassageTitle,
    /// A passage's body, which is also the prompt shown with the passage's choices.
    PassageBody,
    /// An option label of a choice that may be presented next. Whether the option is visible
    /// or enabled then is not evaluated.
    ChoiceLabel,
    /// The reason shown for a disabled option.
    ChoiceReason,
    /// The reply an enabled option of the *current* choice presents when chosen.
    ChoiceReply,
    EndingTitle,
    EndingBody,
}

/// How a media reference would be used, with the visual and music intent of the scene node it
/// came from. A scene node replaces the whole visual target, so its background and every layer
/// are listed; layers of the current scene that the node omits are simply not listed.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum MediaRole {
    Background,
    CharacterLayer {
        id: String,
    },
    /// A `Play` scene directive: the track starts from its beginning.
    MusicPlay,
    /// A `Keep` scene directive: this track, already playing on the path, stays.
    MusicKeep,
    Voice,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ContentDependency {
    pub role: ContentRole,
    pub reference: ContentRef,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct MediaDependency {
    pub role: MediaRole,
    pub reference: MediaRef,
}

/// A logical graph object that a continuation may enter, with the program chunk that holds it.
/// Graphs of the current frames are not listed: the session keeps their chunks resident.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct GraphDependency {
    pub graph: GraphRef,
    pub chunk: ObjectId,
}

/// Why the projection may be missing references. Each entry is a fact about this query, not
/// about the work: an unavailable object may exist and become loadable later.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Truncation {
    /// The walk stopped before visiting more than this many locations.
    Locations { limit: u32 },
    /// The walk stopped before considering more than this many candidate references.
    Candidates { limit: u32 },
    /// The walk stopped before returning more than this many distinct references.
    References { limit: u32 },
    /// This graph was reachable but not loaded, because `limit` other graphs were loaded.
    Graphs { graph: GraphRef, limit: u32 },
    /// A call at this node would exceed the runtime's call depth, so the walk did not enter it.
    CallDepth { graph: GraphRef, node: NodeId },
    /// Automatic nodes lead back to this node on the same path. References on the cycle are
    /// listed, but whether any exit is ever taken is unknown.
    AutomaticCycle { graph: GraphRef, node: NodeId },
    /// The chunk holding `graph` could not be loaded or did not pass the checked decode. The
    /// error is the source's own; nothing downstream of the graph was explored.
    Unavailable {
        graph: GraphRef,
        chunk: ObjectId,
        error: Error,
    },
    /// A reference in the checked program could not be resolved (a missing node, graph or
    /// return continuation).
    Unresolved {
        graph: GraphRef,
        node: Option<NodeId>,
        error: Error,
    },
}

/// What a complete result still did not evaluate. These make the result a superset of what any
/// real continuation presents; they never hide references.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Unknown {
    /// Branch conditions and the visibility or enablement of future options were not evaluated.
    Conditions,
    /// Mutations and option effects were not applied, so later conditions could differ.
    Assignments,
    /// Call arguments were not evaluated.
    CallArguments,
    /// The host may add options and passages at a choice point on the way.
    HostProposals,
}

/// The work a projection did, for hosts that budget or test it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DependencyWork {
    pub locations: u32,
    /// Graphs loaded beyond the current frames' graphs, counting failed attempts.
    pub graphs: u32,
    pub candidates: u32,
}

/// Potential dependencies of the next safe point, tied to the exact commit they were computed
/// from. They are host-owned identities: no bytes, URLs, authorization or decoded handles.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyProjection {
    pub artifact_id: ArtifactId,
    pub cursor: CommitId,
    /// Graph objects first, then the lists below, each sorted and free of repeats.
    pub graphs: Vec<GraphDependency>,
    pub content: Vec<ContentDependency>,
    pub media: Vec<MediaDependency>,
    /// Some continuation stops the music before its safe point. Stopping has no reference.
    pub music_may_stop: bool,
    /// In discovery order. Empty means the walk ran to every safe point and ending.
    pub truncations: Vec<Truncation>,
    pub unknowns: BTreeSet<Unknown>,
    pub work: DependencyWork,
}

impl DependencyProjection {
    /// Whether the walk reached every safe point and ending of the conservative union. A
    /// complete projection with no references means no continuation presents anything more,
    /// for example because the story ends.
    pub fn is_complete(&self) -> bool {
        self.truncations.is_empty()
    }
}

/// One frame of the simulated call stack.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Place {
    graph: GraphRef,
    node: NodeId,
    /// The index in `State::frames` when the frame exists in the state, so that its overlay
    /// applies. Frames entered by the walk have none.
    frame: Option<usize>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Key {
    stack: Vec<Place>,
    /// The scene music on this path, which `Keep` carries on.
    music: Option<MediaRef>,
}

struct Visit {
    parent: Option<usize>,
    key: Key,
}

fn plan_at<'g>(state: &State, graph: &'g Graph, place: &Place) -> Option<Cow<'g, Plan>> {
    match place.frame.and_then(|index| state.frames.get(index)) {
        Some(frame) => frame.overlay.plan(graph, &place.node),
        None => graph.nodes.get(&place.node).map(Cow::Borrowed),
    }
}

/// Whether the reader certainly acts at `point`: the runtime only skips a point that may choose
/// nothing when no option is available, and an unconditional option always is.
fn appears_surely(point: &ChoicePoint) -> bool {
    point.min > 0
        || point.proposals
        || point
            .options
            .iter()
            .any(|option| option.visible_if.is_none() && option.enabled_if.is_none())
}

/// Computes the projection of the checked `state`, which must be the current state of the
/// session that owns `cursor`. Only [`crate::Session`] exposes it, so an unchecked state cannot
/// establish a current interaction.
pub(crate) fn project(
    program: &Program,
    cursor: CommitId,
    state: &State,
    limits: DependencyLimits,
) -> Result<DependencyProjection> {
    let mut walker = Walker {
        program,
        state,
        limits,
        held: state
            .frames
            .iter()
            .map(|frame| frame.graph.clone())
            .collect(),
        graphs: BTreeMap::new(),
        visits: Vec::new(),
        seen: BTreeMap::new(),
        queue: VecDeque::new(),
        listed: BTreeMap::new(),
        content: BTreeSet::new(),
        media: BTreeSet::new(),
        music_may_stop: false,
        truncations: Vec::new(),
        unknowns: BTreeSet::new(),
        work: DependencyWork::default(),
        halted: false,
    };
    let result = walker.run();
    let Walker {
        graphs,
        listed,
        content,
        media,
        music_may_stop,
        truncations,
        unknowns,
        work,
        ..
    } = walker;
    // The borrowed graphs end here, so the cache can drop chunks this query alone kept alive.
    drop(graphs);
    program.trim_chunks();
    result?;
    Ok(DependencyProjection {
        artifact_id: program.artifact_id(),
        cursor,
        graphs: listed
            .into_iter()
            .map(|(graph, chunk)| GraphDependency { graph, chunk })
            .collect(),
        content: content
            .into_iter()
            .map(|(role, reference)| ContentDependency { role, reference })
            .collect(),
        media: media
            .into_iter()
            .map(|(role, reference)| MediaDependency { role, reference })
            .collect(),
        music_may_stop,
        truncations,
        unknowns,
        work,
    })
}

struct Walker<'a> {
    program: &'a Program,
    state: &'a State,
    limits: DependencyLimits,
    /// Graphs of the current frames, which the session already keeps resident.
    held: BTreeSet<GraphRef>,
    /// Graphs requested so far; `None` records a failure or skip so it is never retried.
    graphs: BTreeMap<GraphRef, Option<Arc<Graph>>>,
    visits: Vec<Visit>,
    seen: BTreeMap<Key, usize>,
    queue: VecDeque<usize>,
    listed: BTreeMap<GraphRef, ObjectId>,
    content: BTreeSet<(ContentRole, ContentRef)>,
    media: BTreeSet<(MediaRole, MediaRef)>,
    music_may_stop: bool,
    truncations: Vec<Truncation>,
    unknowns: BTreeSet<Unknown>,
    work: DependencyWork,
    /// Set when a reference bound was hit: nothing more can be recorded.
    halted: bool,
}

impl Walker<'_> {
    fn run(&mut self) -> Result<()> {
        self.seed()?;
        while let Some(index) = self.queue.pop_front() {
            if self.halted {
                break;
            }
            if self.work.locations >= self.limits.locations {
                self.truncate(Truncation::Locations {
                    limit: self.limits.locations,
                });
                break;
            }
            self.work.locations += 1;
            self.visit(index);
        }
        Ok(())
    }

    fn truncate(&mut self, truncation: Truncation) {
        if !self.truncations.contains(&truncation) {
            self.truncations.push(truncation);
        }
    }

    /// Starts the walk at the successors of the current safe point.
    fn seed(&mut self) -> Result<()> {
        let interaction = (Machine {
            program: self.program,
        })
        .interaction(self.state, None)?;
        let state = self.state;
        let Some(frame) = state.frames.last() else {
            // A finished story has no successor. Anything else without a frame is not a state
            // the session can hold.
            return match interaction {
                Interaction::Finished { .. } => Ok(()),
                _ => Err(Error::new("state", "dependencies", "no active frame")),
            };
        };
        let base: Vec<Place> = state
            .frames
            .iter()
            .enumerate()
            .map(|(index, frame)| Place {
                graph: frame.graph.clone(),
                node: frame.node,
                frame: Some(index),
            })
            .collect();
        let music = state.scene.as_ref().and_then(|scene| scene.music.clone());
        let graph = self.program.graph(&frame.graph)?;
        self.graphs.insert(frame.graph.clone(), Some(graph.clone()));
        match interaction {
            Interaction::Finished { .. } => Ok(()),
            Interaction::Advance { .. } => {
                let Some(Plan::Dialogue { next, .. }) = graph.nodes.get(&frame.node) else {
                    return Err(Error::new("state", "dependencies", "dialogue is missing"));
                };
                self.continue_at(None, &base, &music, *next);
                Ok(())
            }
            Interaction::Choose { options, .. } => {
                let enabled: BTreeSet<OptionId> = options
                    .iter()
                    .filter(|option| option.enabled)
                    .map(|option| option.id)
                    .collect();
                let passage = frame
                    .overlay
                    .passage(&graph, &frame.node)
                    .ok_or_else(|| Error::new("state", "dependencies", "not a passage"))?;
                let (index, point) = passage
                    .choice_points
                    .iter()
                    .enumerate()
                    .find(|(_, point)| Some(point.id) == frame.at)
                    .ok_or_else(|| Error::new("state", "dependencies", "no choice point"))?;
                self.seed_choice(&base, &music, &passage, index, point, &enabled);
                Ok(())
            }
        }
    }

    fn seed_choice(
        &mut self,
        base: &[Place],
        music: &Option<MediaRef>,
        passage: &Passage,
        index: usize,
        point: &ChoicePoint,
        enabled: &BTreeSet<OptionId>,
    ) {
        let rejoins = |point: &ChoicePoint| {
            point.options.first().is_some_and(|option| {
                matches!(
                    &option.outcome,
                    Outcome::Local {
                        rejoin: Some(_),
                        ..
                    }
                )
            })
        };
        if point.proposals {
            self.unknowns.insert(Unknown::HostProposals);
        }
        // Choosing nothing is possible when `min` is zero; it follows the first option's rejoin.
        let mut local = point.min == 0;
        let mut rejoin = point.min == 0 && rejoins(point);
        for option in point
            .options
            .iter()
            .filter(|option| enabled.contains(&option.id))
        {
            if !option.effects.is_empty() {
                self.unknowns.insert(Unknown::Assignments);
            }
            match &option.outcome {
                Outcome::Local {
                    reply,
                    rejoin: anchor,
                } => {
                    local = true;
                    rejoin |= anchor.is_some();
                    if let Some(reply) = reply {
                        self.note_content(ContentRole::ChoiceReply, &reply.unit);
                    }
                }
                Outcome::Branch { target } => self.continue_at(None, base, music, *target),
            }
        }
        if local {
            rejoin |= passage.choice_points[index + 1..]
                .iter()
                .take_while(|point| point.min == 0 && !point.proposals)
                .any(rejoins);
            if rejoin && let Some(body) = &passage.body {
                self.note_content(ContentRole::PassageBody, &body.unit);
            }
            self.arrive(None, base, music, passage, false, index + 1);
        }
    }

    /// Queues the place that continues `stack` at `node` of its top frame.
    fn continue_at(
        &mut self,
        parent: Option<usize>,
        stack: &[Place],
        music: &Option<MediaRef>,
        node: NodeId,
    ) {
        let mut stack = stack.to_vec();
        if let Some(top) = stack.last_mut() {
            top.node = node;
        }
        self.enqueue(parent, stack, music.clone());
    }

    fn enqueue(&mut self, parent: Option<usize>, stack: Vec<Place>, music: Option<MediaRef>) {
        let key = Key { stack, music };
        if let Some(&existing) = self.seen.get(&key) {
            // Reaching a place again is convergence, unless it is an ancestor on this path.
            let mut cursor = parent;
            while let Some(index) = cursor {
                if index == existing {
                    if let Some(top) = key.stack.last() {
                        let truncation = Truncation::AutomaticCycle {
                            graph: top.graph.clone(),
                            node: top.node,
                        };
                        self.truncate(truncation);
                    }
                    break;
                }
                cursor = self.visits.get(index).and_then(|visit| visit.parent);
            }
            return;
        }
        let index = self.visits.len();
        self.seen.insert(key.clone(), index);
        self.visits.push(Visit { parent, key });
        self.queue.push_back(index);
    }

    fn visit(&mut self, index: usize) {
        let Some(Visit { key, .. }) = self.visits.get(index) else {
            return;
        };
        let (stack, music) = (key.stack.clone(), key.music.clone());
        let Some(top) = stack.last() else {
            return;
        };
        let Some(graph) = self.graph(&top.graph) else {
            return;
        };
        let state = self.state;
        let Some(plan) = plan_at(state, &graph, top) else {
            self.truncate(Truncation::Unresolved {
                graph: top.graph.clone(),
                node: Some(top.node),
                error: Error::new("state", top.node.to_string(), "unknown checked node"),
            });
            return;
        };
        let parent = Some(index);
        match &*plan {
            Plan::Passage(passage) => self.arrive(parent, &stack, &music, passage, true, 0),
            Plan::Dialogue {
                speaker,
                line,
                voice,
                ..
            } => {
                if let Some(speaker) = speaker {
                    self.note_content(ContentRole::DialogueSpeaker, speaker);
                }
                self.note_content(ContentRole::DialogueLine, &line.unit);
                if let Some(voice) = voice {
                    self.note_media(MediaRole::Voice, voice);
                }
            }
            Plan::Scene {
                background,
                layers,
                music: action,
                next,
            } => {
                if let Some(background) = background {
                    self.note_media(MediaRole::Background, background);
                }
                for layer in layers {
                    self.note_media(
                        MediaRole::CharacterLayer {
                            id: layer.id.clone(),
                        },
                        &layer.asset,
                    );
                }
                let music = match action {
                    MusicAction::Keep => {
                        if let Some(kept) = &music {
                            self.note_media(MediaRole::MusicKeep, kept);
                        }
                        music
                    }
                    MusicAction::Play { track } => {
                        self.note_media(MediaRole::MusicPlay, track);
                        Some(track.clone())
                    }
                    MusicAction::Stop => {
                        self.music_may_stop = true;
                        None
                    }
                };
                self.continue_at(parent, &stack, &music, *next);
            }
            Plan::Mutate { assignments, next } => {
                if !assignments.is_empty() {
                    self.unknowns.insert(Unknown::Assignments);
                }
                self.continue_at(parent, &stack, &music, *next);
            }
            Plan::Branch {
                when_true,
                when_false,
                ..
            } => {
                self.unknowns.insert(Unknown::Conditions);
                self.continue_at(parent, &stack, &music, *when_true);
                self.continue_at(parent, &stack, &music, *when_false);
            }
            Plan::Call {
                target, arguments, ..
            } => {
                if !arguments.is_empty() {
                    self.unknowns.insert(Unknown::CallArguments);
                }
                if stack.len() >= MAX_CALL_DEPTH {
                    self.truncate(Truncation::CallDepth {
                        graph: top.graph.clone(),
                        node: top.node,
                    });
                    return;
                }
                let callee = match self.program.resolve_call(&top.graph, target) {
                    Ok(callee) => callee,
                    Err(error) => {
                        self.truncate(Truncation::Unresolved {
                            graph: top.graph.clone(),
                            node: Some(top.node),
                            error,
                        });
                        return;
                    }
                };
                self.list_graph(&callee);
                let Some(entry) = self.graph(&callee).map(|graph| graph.header.entry) else {
                    return;
                };
                let mut stack = stack.clone();
                stack.push(Place {
                    graph: callee,
                    node: entry,
                    frame: None,
                });
                self.enqueue(parent, stack, music);
            }
            Plan::Return { outcome } => {
                let mut stack = stack.clone();
                stack.pop();
                let Some(caller) = stack.last() else {
                    let ending = self
                        .program
                        .manifest()
                        .product
                        .endings
                        .get(outcome)
                        .cloned()
                        .unwrap_or_default();
                    if let Some(title) = &ending.title {
                        self.note_content(ContentRole::EndingTitle, title);
                    }
                    if let Some(body) = &ending.body {
                        self.note_content(ContentRole::EndingBody, &body.unit);
                    }
                    return;
                };
                let Some(graph) = self.graph(&caller.graph) else {
                    return;
                };
                let resumed = match plan_at(state, &graph, caller).as_deref() {
                    Some(Plan::Call { on_return, .. }) => on_return.get(outcome).copied(),
                    _ => None,
                };
                match resumed {
                    Some(node) => self.continue_at(parent, &stack, &music, node),
                    None => self.truncate(Truncation::Unresolved {
                        graph: caller.graph.clone(),
                        node: Some(caller.node),
                        error: Error::new("state", "return", "missing outcome continuation"),
                    }),
                }
            }
        }
    }

    /// Arrives at `passage` from choice point `first_point` on. Entering a passage lists its
    /// title and body (`header`); continuing after a local choice does not. The walk stops at
    /// the first choice point that certainly appears, else the passage runs on to its `next`.
    fn arrive(
        &mut self,
        parent: Option<usize>,
        stack: &[Place],
        music: &Option<MediaRef>,
        passage: &Passage,
        header: bool,
        first_point: usize,
    ) {
        if header {
            if let Some(title) = &passage.title {
                self.note_content(ContentRole::PassageTitle, title);
            }
            if let Some(body) = &passage.body {
                self.note_content(ContentRole::PassageBody, &body.unit);
            }
        }
        for point in passage.choice_points.iter().skip(first_point) {
            if point.proposals {
                self.unknowns.insert(Unknown::HostProposals);
            }
            for option in &point.options {
                if option.visible_if.is_some() || option.enabled_if.is_some() {
                    self.unknowns.insert(Unknown::Conditions);
                }
                if let Some(label) = &option.label {
                    self.note_content(ContentRole::ChoiceLabel, label);
                }
                if let Some(reason) = &option.reason {
                    self.note_content(ContentRole::ChoiceReason, reason);
                }
            }
            if appears_surely(point) {
                return;
            }
        }
        if let Some(next) = passage.next {
            self.continue_at(parent, stack, music, next);
        }
    }

    /// Counts one candidate; false when the bound stops the walk.
    fn candidate(&mut self) -> bool {
        if self.halted {
            return false;
        }
        if self.work.candidates >= self.limits.candidates {
            self.halted = true;
            self.truncate(Truncation::Candidates {
                limit: self.limits.candidates,
            });
            return false;
        }
        self.work.candidates += 1;
        true
    }

    /// Whether one more distinct reference fits; stops the walk when it does not.
    fn has_room(&mut self) -> bool {
        let distinct = self.listed.len() + self.content.len() + self.media.len();
        if distinct >= self.limits.references as usize {
            self.halted = true;
            self.truncate(Truncation::References {
                limit: self.limits.references,
            });
            return false;
        }
        true
    }

    fn note_content(&mut self, role: ContentRole, reference: &ContentRef) {
        if self.candidate() && !self.content.contains(&(role, reference.clone())) && self.has_room()
        {
            self.content.insert((role, reference.clone()));
        }
    }

    fn note_media(&mut self, role: MediaRole, reference: &MediaRef) {
        if self.candidate()
            && !self.media.contains(&(role.clone(), reference.clone()))
            && self.has_room()
        {
            self.media.insert((role, reference.clone()));
        }
    }

    fn chunk_of(&self, graph: &GraphRef) -> Result<ObjectId> {
        let manifest = self.program.manifest();
        manifest
            .graphs
            .get(graph)
            .and_then(|entry| manifest.chunks.get(entry.chunk as usize))
            .copied()
            .ok_or_else(|| Error::new("reference", graph.label(), "unknown graph"))
    }

    /// Records that `graph` may be entered, with the chunk that holds it. The manifest answers
    /// this without loading anything.
    fn list_graph(&mut self, graph: &GraphRef) {
        if self.halted || self.held.contains(graph) || self.listed.contains_key(graph) {
            return;
        }
        match self.chunk_of(graph) {
            Ok(chunk) => {
                if self.candidate() && self.has_room() {
                    self.listed.insert(graph.clone(), chunk);
                }
            }
            Err(error) => self.truncate(Truncation::Unresolved {
                graph: graph.clone(),
                node: None,
                error,
            }),
        }
    }

    /// Loads `graph` through the program's checked cache, once per query and within budget.
    fn graph(&mut self, graph: &GraphRef) -> Option<Arc<Graph>> {
        if let Some(known) = self.graphs.get(graph) {
            return known.clone();
        }
        let held = self.held.contains(graph);
        let loaded = if !held && self.work.graphs >= self.limits.graphs {
            self.truncate(Truncation::Graphs {
                graph: graph.clone(),
                limit: self.limits.graphs,
            });
            None
        } else {
            match self.chunk_of(graph) {
                Err(error) => {
                    self.truncate(Truncation::Unresolved {
                        graph: graph.clone(),
                        node: None,
                        error,
                    });
                    None
                }
                Ok(chunk) => {
                    if !held {
                        self.work.graphs += 1;
                    }
                    match self.program.graph(graph) {
                        Ok(loaded) => Some(loaded),
                        Err(error) => {
                            self.truncate(Truncation::Unavailable {
                                graph: graph.clone(),
                                chunk,
                                error,
                            });
                            None
                        }
                    }
                }
            }
        };
        self.graphs.insert(graph.clone(), loaded.clone());
        loaded
    }
}
