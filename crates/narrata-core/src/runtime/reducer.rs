use std::{collections::BTreeMap, sync::Arc};

use thiserror::Error;

use crate::{
    CommitId,
    identity::{ExecutionId, LocalId},
    limits::MacrostepLimits,
    program::CheckedProgram,
    scene::SceneState,
    value::Value,
    version::{SEMANTICS_V0, SNAPSHOT_SCHEMA_V0},
};

use super::{
    CheckedRuntimeInput, FrameStateV0, RuntimeStateV0, RuntimeStatusV0, TransitionRunner,
    TransitionStartError, Turn, VmStateV0,
};

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RuntimeInitError {
    #[error("checked Program entry Flow is unavailable")]
    MissingEntryFlow,
    #[error("entry Flow unexpectedly declares parameters")]
    EntryFlowParameters,
}

pub fn new_execution(
    program: &CheckedProgram,
    execution_id: ExecutionId,
) -> Result<RuntimeStateV0, RuntimeInitError> {
    let artifact = program.artifact();
    let entry = program
        .flow(artifact.entry_flow)
        .ok_or(RuntimeInitError::MissingEntryFlow)?;
    if !entry.parameters.is_empty() {
        return Err(RuntimeInitError::EntryFlowParameters);
    }
    let globals = artifact
        .globals
        .iter()
        .map(|decl| (decl.id, decl.default.clone()))
        .collect::<BTreeMap<_, _>>();
    let locals = entry
        .locals
        .iter()
        .map(|decl| (decl.id, decl.default.clone()))
        .collect::<BTreeMap<LocalId, Value>>();
    let has_statechart = program.statechart().is_some();
    let snapshot_schema = program.snapshot_schema();
    let scene =
        (snapshot_schema == SNAPSHOT_SCHEMA_V0 || program.uses_scene()).then(SceneState::default);
    Ok(RuntimeStateV0 {
        snapshot_schema,
        semantics_version: SEMANTICS_V0,
        execution_id,
        program_artifact_id: program.artifact_id(),
        turn: Turn(0),
        interaction_counter: 0,
        globals,
        scene,
        statechart: has_statechart.then(Default::default),
        status: if has_statechart {
            RuntimeStatusV0::StatechartStable
        } else {
            RuntimeStatusV0::Ready {
                vm: VmStateV0 {
                    frames: vec![FrameStateV0 {
                        flow: entry.id,
                        instruction: entry.entry,
                        return_to: None,
                        locals,
                        evaluation_stack: Vec::new(),
                    }],
                },
            }
        },
    })
}

pub fn begin_transition(
    program: Arc<CheckedProgram>,
    parent: Arc<RuntimeStateV0>,
    input: CheckedRuntimeInput,
    limits: MacrostepLimits,
) -> Result<TransitionRunner, TransitionStartError> {
    TransitionRunner::begin(program, parent, input, limits, None)
}

pub fn begin_transition_with_parent_commit(
    program: Arc<CheckedProgram>,
    parent: Arc<RuntimeStateV0>,
    parent_commit: CommitId,
    input: CheckedRuntimeInput,
    limits: MacrostepLimits,
) -> Result<TransitionRunner, TransitionStartError> {
    TransitionRunner::begin(program, parent, input, limits, Some(parent_commit))
}
