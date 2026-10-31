use super::{DeletedId, DiscardedFrameV1, EvolutionDescriptorV1, RecoveryLossV1};
use crate::{
    AuthoredId, Error, Frame, Lookup, Overlay, Program, Result, State, decode_state,
    runtime::Machine, view::Interaction,
};
use std::collections::BTreeSet;

impl EvolutionDescriptorV1 {
    /// A deterministic transformation of the stored state, never a replay of past effects.
    /// A registered descriptor has already been checked against both full artifacts.
    pub fn migrate_state(
        &self,
        source: &Program,
        target: &Program,
        state: &State,
    ) -> Result<(State, Option<RecoveryLossV1>)> {
        self.validate(source, target)?;
        self.transform(source, target, state)
    }

    pub(crate) fn transform(
        &self,
        source: &Program,
        target: &Program,
        state: &State,
    ) -> Result<(State, Option<RecoveryLossV1>)> {
        if source.artifact_id() != self.source_artifact()
            || target.artifact_id() != self.target_artifact()
        {
            return Err(Error::new(
                "artifact",
                "migration",
                "descriptor artifact mismatch",
            ));
        }
        decode_state(source, &state.envelope())?;
        let mut deleted = BTreeSet::new();
        for frame in &state.frames {
            // Dynamic identities are checked with their overlay, never static lookup.
            if !frame.overlay.nodes.contains_key(&frame.node) {
                position(target, DeletedId::Node(frame.node), &mut deleted)?;
            }
            if let Some(at) = frame.at {
                let dynamic = frame
                    .overlay
                    .nodes
                    .values()
                    .any(|p| p.choice_points.iter().any(|p| p.id == at));
                if !dynamic {
                    position(target, DeletedId::ChoicePoint(at), &mut deleted)?;
                }
            }
        }
        if let Some(finished) = &state.finished {
            position(target, DeletedId::Node(finished.node), &mut deleted)?;
        }
        if deleted.is_empty() {
            return Ok((checked(target, state)?, None));
        }
        let mut recovery_target = None;
        for id in &deleted {
            let mapping = self
                .recoveries()
                .iter()
                .find(|r| r.deleted == *id)
                .ok_or_else(|| {
                    Error::new(
                        "unmapped_continuation",
                        "migration",
                        "a deleted continuation has no recovery declaration",
                    )
                })?;
            if recovery_target.is_some_and(|target| target != mapping.target) {
                return Err(Error::new(
                    "conflicting_recovery",
                    "migration",
                    "deleted continuations have different recovery targets",
                ));
            }
            recovery_target = Some(mapping.target);
        }
        let destination = recovery_target
            .ok_or_else(|| Error::new("recovery", "migration", "missing recovery target"))?;
        let product = &target.manifest().product;
        let graph = target.graph(&product.entry)?;
        let next_instance = state.next_instance.checked_add(1).ok_or_else(|| {
            Error::new("instance_limit", "migration", "recovery instance overflow")
        })?;
        let candidate = State {
            shared: state.shared.clone(),
            next_instance,
            finished: None,
            scene: None,
            frames: vec![Frame {
                graph: product.entry.clone(),
                node: destination.node,
                at: Some(destination.choice_point),
                instance: state.next_instance,
                parameters: product.arguments.clone(),
                locals: graph.header.locals.clone(),
                overlay: Overlay::default(),
            }],
        };
        // Shared key and type compatibility is proved by target decode, including additions
        // and removals. A declaration cannot assign or discard shared variable values.
        let candidate = checked(target, &candidate)?;
        let discarded_frames = state
            .frames
            .iter()
            .map(|frame| {
                let mut ids = BTreeSet::new();
                for (node, passage) in &frame.overlay.nodes {
                    ids.insert(AuthoredId::Node(*node));
                    for point in &passage.choice_points {
                        ids.insert(AuthoredId::ChoicePoint(point.id));
                        for option in &point.options {
                            ids.insert(AuthoredId::Option(option.id));
                        }
                    }
                }
                for options in frame.overlay.options.values() {
                    for option in options {
                        ids.insert(AuthoredId::Option(option.id));
                    }
                }
                DiscardedFrameV1 {
                    instance: frame.instance,
                    node: frame.node,
                    local_keys: frame.locals.keys().cloned().collect(),
                    parameter_keys: frame.parameters.keys().cloned().collect(),
                    overlay_ids: ids.into_iter().collect(),
                    pending_choice: frame.at,
                }
            })
            .collect();
        let loss = RecoveryLossV1 {
            tombstones: deleted.into_iter().collect(),
            discarded_frames,
            cleared_finished: state.finished.as_ref().map(|f| f.node),
            retained_shared_keys: state.shared.keys().cloned().collect(),
            reset_local_keys: graph.header.locals.keys().cloned().collect(),
            target: destination,
        };
        Ok((candidate, Some(loss)))
    }
}
fn position(target: &Program, id: DeletedId, deleted: &mut BTreeSet<DeletedId>) -> Result<()> {
    match target.lookup(id.authored())? {
        Lookup::Deleted => {
            deleted.insert(id);
        }
        Lookup::Live { .. } => {}
        Lookup::Unknown => {
            return Err(Error::new(
                "unknown_id",
                "migration",
                "unknown continuation is not a tombstone",
            ));
        }
    }
    Ok(())
}
fn checked(target: &Program, state: &State) -> Result<State> {
    let (decoded, _) = decode_state(target, &state.envelope())?;
    if decoded.encode() != state.encode() {
        return Err(Error::new(
            "canonical",
            "migration",
            "target state roundtrip differs",
        ));
    }
    if let Interaction::Choose {
        min,
        proposals,
        options,
        ..
    } = (Machine { program: target }).interaction(&decoded, None)?
        && !proposals
        && (options.iter().filter(|o| o.enabled).count() < usize::from(min)
            || (min > 0 && options.iter().all(|o| !o.enabled)))
    {
        return Err(Error::new(
            "unavailable_interaction",
            "migration",
            "target interaction cannot continue",
        ));
    }
    Ok(decoded)
}
