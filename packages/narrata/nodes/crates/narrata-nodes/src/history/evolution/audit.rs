use super::{
    DivergenceReason, EvolutionRegistry, LocationV1, MigrationId, PathComparisonV1, PathLimits,
    UnverifiedReason, differing_fields,
};
use crate::{
    ArtifactId, Commit, CommitId, Error, Input, ObjectId, Program, Result, State, decode_input,
    decode_state,
    history::NodeDomain,
    plan::Outcome,
    runtime::trace::{TraceEvent, TraceLocation},
    wire,
};
use narrata_history::migration::{MIGRATION_INPUT_KIND, MigrationInput};
use narrata_history::{DomainKinds, History, HistoryError, Object};
use narrata_storage::StorageBackend;
use std::collections::BTreeSet;

struct Budget {
    limits: PathLimits,
    bytes: usize,
    steps: usize,
    seen: BTreeSet<narrata_history::ObjectId>,
}
impl Budget {
    fn read<B: StorageBackend>(
        &mut self,
        history: &History<B, DomainKinds<NodeDomain>>,
        id: narrata_history::ObjectId,
        kind: Option<u16>,
    ) -> Result<Object> {
        let object = history.reader().require(id, kind).map_err(|e| match e {
            HistoryError::MissingObject(id) | HistoryError::HistoryTruncated(id) => Error::new(
                "needs_object",
                id.to_string(),
                "required history object is unavailable",
            ),
            _ => Error::new("invalid_save", id.to_string(), "invalid history object"),
        })?;
        if self.seen.insert(id) {
            self.bytes = self.bytes.saturating_add(object.bytes().len());
        }
        if self.bytes > self.limits.object_bytes {
            return Err(Error::new("path_limit", "audit", "object byte limit"));
        }
        Ok(object)
    }
    fn trace(&mut self, events: &[TraceEvent]) -> Result<()> {
        self.steps = self
            .steps
            .saturating_add(events.iter().filter(|e| e.is_visit()).count());
        if self.steps > self.limits.automatic_steps {
            return Err(Error::new("path_limit", "audit", "automatic step limit"));
        }
        Ok(())
    }
}
fn kid(id: CommitId) -> narrata_history::ObjectId {
    narrata_history::ObjectId::from_bytes(*id.as_bytes())
}
fn oid(id: ObjectId) -> narrata_history::ObjectId {
    narrata_history::ObjectId::from_bytes(*id.as_bytes())
}
fn unverified(
    reason: UnverifiedReason,
    baseline: Option<CommitId>,
    checked_through: Option<CommitId>,
) -> PathComparisonV1 {
    PathComparisonV1::Unverified {
        reason,
        baseline,
        checked_through,
    }
}

impl EvolutionRegistry {
    /// Audits every readable source segment, then compares only the latest segment on the
    /// requested target. A committed migration is a baseline, not an ordinary input.
    pub fn compare_path<B: StorageBackend>(
        &self,
        history: &History<B, DomainKinds<NodeDomain>>,
        source: CommitId,
        target: ArtifactId,
        limits: PathLimits,
    ) -> Result<PathComparisonV1> {
        match self.compare(history, source, target, limits) {
            Err(error) if error.code == "path_limit" => {
                Ok(unverified(UnverifiedReason::Limit, None, None))
            }
            result => result,
        }
    }
    fn compare<B: StorageBackend>(
        &self,
        history: &History<B, DomainKinds<NodeDomain>>,
        source: CommitId,
        target: ArtifactId,
        limits: PathLimits,
    ) -> Result<PathComparisonV1> {
        let limits = limits.bounded();
        let mut budget = Budget {
            limits,
            bytes: 0,
            steps: 0,
            seen: BTreeSet::new(),
        };
        let mut path = Vec::new();
        let mut next = Some(source);
        while let Some(id) = next {
            let object = match budget.read(history, kid(id), Some(wire::KIND_COMMIT)) {
                Err(e) if e.code == "needs_object" && id != source => {
                    return Ok(unverified(
                        UnverifiedReason::MissingAncestor,
                        Some(id),
                        None,
                    ));
                }
                other => other?,
            };
            let header = Commit::decode(object.bytes())?;
            next = header.parent;
            path.push((id, header));
            if path.len() > limits.transitions.saturating_add(65) {
                return Ok(unverified(UnverifiedReason::Limit, None, None));
            }
        }
        path.reverse();
        let latest = path
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| pair[0].1.artifact != pair[1].1.artifact)
            .map(|(i, _)| i + 1)
            .next_back()
            .unwrap_or(0);
        let target_program = self.program(target)?;
        let mut previous: Option<(CommitId, &Commit, State)> = None;
        let mut interim_parent = None;
        let mut shadow: Option<State> = None;
        let mut conclusion = PathComparisonV1::Equivalent;
        let mut transitions = 0;
        let mut migration_steps = 0;
        for (index, (id, header)) in path.iter().enumerate() {
            let source_program = match self.program(header.artifact) {
                Ok(p) => p,
                Err(_) => {
                    return Ok(unverified(
                        UnverifiedReason::MissingArtifact,
                        Some(*id),
                        previous.as_ref().map(|p| p.0),
                    ));
                }
            };
            let object = budget.read(history, oid(header.state), Some(wire::KIND_STATE))?;
            let (recorded, _) = decode_state(source_program, object.bytes())?;
            let mut input = None;
            let mut input_parent = None;
            let old_events;
            if let Some((parent_id, parent_header, parent_state)) = &previous {
                if header.parent != Some(*parent_id)
                    || parent_header.depth.checked_add(1) != Some(header.depth)
                {
                    return Err(Error::new(
                        "invalid_save",
                        id.to_string(),
                        "invalid parent/depth link",
                    ));
                }
                let input_id = header
                    .input
                    .ok_or_else(|| Error::new("invalid_save", id.to_string(), "missing input"))?;
                let object = budget.read(history, oid(input_id), None)?;
                if object.kind() == MIGRATION_INPUT_KIND {
                    migration_steps += 1;
                    if migration_steps > 64 {
                        return Ok(unverified(
                            UnverifiedReason::Limit,
                            Some(*id),
                            Some(*parent_id),
                        ));
                    }
                    let edge = MigrationInput::from_object(&object).map_err(|_| {
                        Error::new("invalid_save", id.to_string(), "invalid migration input")
                    })?;
                    if edge.from.as_bytes() != parent_header.artifact.as_bytes()
                        || edge.to.as_bytes() != header.artifact.as_bytes()
                    {
                        return Err(Error::new(
                            "invalid_save",
                            id.to_string(),
                            "migration links differ",
                        ));
                    }
                    let migration_id = MigrationId::from_bytes(*edge.id.as_bytes());
                    let Some(descriptor) = self.descriptor(migration_id) else {
                        return Ok(unverified(
                            UnverifiedReason::MissingDescriptor,
                            Some(*id),
                            Some(*parent_id),
                        ));
                    };
                    if descriptor.id() != migration_id
                        || descriptor.source_artifact() != parent_header.artifact
                        || descriptor.target_artifact() != header.artifact
                    {
                        return Err(Error::new(
                            "invalid_save",
                            id.to_string(),
                            "migration descriptor identity differs",
                        ));
                    }
                    let (migrated, _) = descriptor.transform(
                        self.program(parent_header.artifact)?,
                        source_program,
                        parent_state,
                    )?;
                    if migrated.encode() != recorded.encode() {
                        return Err(Error::new(
                            "invalid_save",
                            id.to_string(),
                            "migration state differs from its descriptor",
                        ));
                    }
                    old_events = Vec::new();
                } else {
                    transitions += 1;
                    if transitions > limits.transitions {
                        return Ok(unverified(
                            UnverifiedReason::Limit,
                            Some(path[latest].0),
                            Some(*parent_id),
                        ));
                    }
                    if parent_header.artifact != header.artifact
                        || object.kind() != wire::KIND_INPUT
                    {
                        return Err(Error::new(
                            "invalid_save",
                            id.to_string(),
                            "ordinary input changes artifact",
                        ));
                    }
                    let parsed =
                        match decode_input(source_program, parent_id, parent_state, object.bytes())
                        {
                            Err(error) if error.code == "derived_id" => {
                                let old_parent = interim_parent.ok_or_else(|| {
                                    Error::new("invalid_save", "proposal", "missing interim parent")
                                })?;
                                let input = decode_input(
                                    source_program,
                                    &old_parent,
                                    parent_state,
                                    object.bytes(),
                                )?
                                .0;
                                input_parent = Some(old_parent);
                                input
                            }
                            other => {
                                input_parent = Some(*parent_id);
                                other?.0
                            }
                        };
                    let step = source_program.trace_apply(
                        &input_parent
                            .ok_or_else(|| Error::new("invalid_save", "input", "missing parent"))?,
                        parent_state,
                        &parsed,
                    )?;
                    budget.trace(&step.events)?;
                    if step.state.encode() != recorded.encode() {
                        return Err(Error::new(
                            "invalid_save",
                            id.to_string(),
                            "source replay differs from stored state",
                        ));
                    }
                    old_events = step.events;
                    input = Some(parsed);
                }
            } else {
                let initial = source_program.trace_initial()?;
                budget.trace(&initial.events)?;
                if initial.state.encode() != recorded.encode() {
                    return Err(Error::new(
                        "invalid_save",
                        id.to_string(),
                        "root differs from machine initial state",
                    ));
                }
                old_events = initial.events;
            }
            // Keep auditing the source after a target divergence: a later corrupt source
            // record is InvalidSave, never evidence about changed rules.
            if index >= latest && conclusion == PathComparisonV1::Equivalent {
                if index == latest && latest > 0 {
                    match decode_state(target_program, &recorded.envelope()) {
                        Ok((baseline, _)) => {
                            let fields = differing_fields(&recorded, &baseline);
                            if fields.is_empty() {
                                shadow = Some(baseline);
                            } else {
                                conclusion =
                                    unverified(UnverifiedReason::Baseline, Some(*id), None);
                            }
                        }
                        Err(_) => {
                            conclusion = unverified(UnverifiedReason::Baseline, Some(*id), None)
                        }
                    }
                } else {
                    let step = if index == 0 {
                        target_program.trace_initial()
                    } else {
                        let parent = shadow.as_ref().ok_or_else(|| {
                            Error::new("invalid_save", "shadow", "missing shadow baseline")
                        })?;
                        let original = input.as_ref().ok_or_else(|| {
                            Error::new("invalid_save", "shadow", "missing ordinary input")
                        })?;
                        let shadow_input = match shadow_input(target_program, parent, original) {
                            Ok(input) => input,
                            Err(_) => {
                                conclusion = divergence(
                                    *id,
                                    header,
                                    DivergenceReason::InvalidInput,
                                    source_program,
                                    &recorded,
                                    target_program,
                                    Some(parent),
                                    vec!["input".into()],
                                );
                                let mut interim = header.clone();
                                interim.parent = interim_parent;
                                interim_parent = Some(CommitId::from_bytes(
                                    *wire::id_of(
                                        wire::KIND_COMMIT,
                                        &wire::encode_interim_commit(&interim),
                                    )
                                    .as_bytes(),
                                ));
                                previous = Some((*id, header, recorded));
                                continue;
                            }
                        };
                        target_program.trace_apply(
                            &input_parent.ok_or_else(|| {
                                Error::new("invalid_save", "shadow", "missing original parent")
                            })?,
                            parent,
                            &shadow_input,
                        )
                    };
                    match step {
                        Err(_) => {
                            conclusion = divergence(
                                *id,
                                header,
                                if index == 0 {
                                    DivergenceReason::InitialState
                                } else {
                                    DivergenceReason::Execution
                                },
                                source_program,
                                &recorded,
                                target_program,
                                shadow.as_ref(),
                                vec!["execution".into()],
                            )
                        }
                        Ok(step) => {
                            budget.trace(&step.events)?;
                            let fields = differing_fields(&recorded, &step.state);
                            if control(&old_events) != control(&step.events) {
                                conclusion = divergence(
                                    *id,
                                    header,
                                    DivergenceReason::ControlTrace,
                                    source_program,
                                    &recorded,
                                    target_program,
                                    Some(&step.state),
                                    vec!["control_trace".into()],
                                );
                            } else if !fields.is_empty() {
                                conclusion = divergence(
                                    *id,
                                    header,
                                    if index == 0 {
                                        DivergenceReason::InitialState
                                    } else {
                                        DivergenceReason::SemanticState
                                    },
                                    source_program,
                                    &recorded,
                                    target_program,
                                    Some(&step.state),
                                    fields,
                                );
                            } else {
                                shadow = Some(step.state);
                            }
                        }
                    }
                }
            }
            let mut interim = header.clone();
            interim.parent = interim_parent;
            interim_parent = Some(CommitId::from_bytes(
                *wire::id_of(wire::KIND_COMMIT, &wire::encode_interim_commit(&interim)).as_bytes(),
            ));
            previous = Some((*id, header, recorded));
        }
        Ok(conclusion)
    }
}
#[allow(clippy::too_many_arguments)]
fn divergence(
    id: CommitId,
    header: &Commit,
    reason: DivergenceReason,
    old_program: &Program,
    old: &State,
    new_program: &Program,
    new: Option<&State>,
    differing_fields: Vec<String>,
) -> PathComparisonV1 {
    PathComparisonV1::Diverged {
        source_commit: id,
        input_id: header.input,
        reason,
        old_location: LocationV1::of(old_program, old),
        new_location: new.and_then(|s| LocationV1::of(new_program, s)),
        differing_fields,
    }
}
fn shadow_input(program: &Program, state: &State, input: &Input) -> Result<Input> {
    let Input::Choose {
        choice_point,
        options,
    } = input
    else {
        return Ok(input.clone());
    };
    let frame = state
        .frames
        .last()
        .ok_or_else(|| Error::new("action", "shadow", "finished"))?;
    let graph = program.graph(&frame.graph)?;
    let passage = frame
        .overlay
        .passage(&graph, &frame.node)
        .ok_or_else(|| Error::new("action", "shadow", "not a passage"))?;
    let point = passage
        .choice_points
        .iter()
        .find(|p| p.id == *choice_point)
        .ok_or_else(|| Error::new("action", "shadow", "choice deleted"))?;
    let selected: BTreeSet<_> = options.iter().copied().collect();
    let ordered: Vec<_> = point
        .options
        .iter()
        .filter(|o| selected.contains(&o.id))
        .map(|o| o.id)
        .collect();
    if ordered.len() != options.len() {
        return Err(Error::new("action", "shadow", "selected option deleted"));
    }
    Ok(Input::Choose {
        choice_point: *choice_point,
        options: ordered,
    })
}

#[derive(Eq, PartialEq)]
enum Control {
    Visit(TraceLocation),
    Selected(
        TraceLocation,
        crate::ChoicePointId,
        crate::OptionId,
        Option<crate::NodeId>,
    ),
    Branch(TraceLocation, crate::NodeId),
    Call(TraceLocation, TraceLocation),
    Return(TraceLocation, String, Option<TraceLocation>),
    Finished(TraceLocation, String),
}
fn control(events: &[TraceEvent]) -> Vec<Control> {
    events
        .iter()
        .filter_map(|e| {
            let location = e.location.clone();
            if e.is_visit() {
                Some(Control::Visit(location))
            } else if let Some((point, option, outcome)) = e.selection() {
                Some(Control::Selected(
                    location,
                    point,
                    option,
                    match outcome {
                        Outcome::Local { .. } => None,
                        Outcome::Branch { target } => Some(*target),
                    },
                ))
            } else if let Some(target) = e.branch_target() {
                Some(Control::Branch(location, target))
            } else if let Some(target) = e.call_target() {
                Some(Control::Call(location, target.clone()))
            } else if let Some((outcome, continuation)) = e.returned() {
                Some(Control::Return(
                    location,
                    outcome.into(),
                    continuation.cloned(),
                ))
            } else if let Some((outcome, _)) = e.ending() {
                Some(Control::Finished(location, outcome.into()))
            } else {
                None
            }
        })
        .collect()
}
