use std::collections::BTreeMap;

use crate::{
    identity::{ExecutionId, GlobalId, ProgramArtifactId},
    value::Value,
    version::SemanticsVersion,
};

use super::{FrameStateV0, PendingInteractionV0, VmStateV0};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Turn(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeStateV0 {
    pub semantics_version: SemanticsVersion,
    pub execution_id: ExecutionId,
    pub program_artifact_id: ProgramArtifactId,
    pub turn: Turn,
    pub interaction_counter: u64,
    pub globals: BTreeMap<GlobalId, Value>,
    pub status: RuntimeStatusV0,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeStatusV0 {
    Ready {
        vm: VmStateV0,
    },
    Awaiting {
        vm: VmStateV0,
        pending: PendingInteractionV0,
    },
    Finished {
        result: Value,
        final_frames: Vec<FrameStateV0>,
    },
}

impl RuntimeStateV0 {
    pub fn pending(&self) -> Option<&PendingInteractionV0> {
        match &self.status {
            RuntimeStatusV0::Awaiting { pending, .. } => Some(pending),
            RuntimeStatusV0::Ready { .. } | RuntimeStatusV0::Finished { .. } => None,
        }
    }

    pub fn vm(&self) -> Option<&VmStateV0> {
        match &self.status {
            RuntimeStatusV0::Ready { vm } | RuntimeStatusV0::Awaiting { vm, .. } => Some(vm),
            RuntimeStatusV0::Finished { .. } => None,
        }
    }
}
