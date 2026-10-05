use std::collections::BTreeMap;

use crate::{
    identity::{ExecutionId, GlobalId, ProgramArtifactId},
    scene::SceneState,
    statechart::StatechartStateV0,
    value::Value,
    version::{SemanticsVersion, SnapshotSchemaVersion},
};

use super::{FrameStateV0, PendingEffectV0, PendingInteractionV0, VmStateV0};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Turn(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeStateV0 {
    /// Follows the Program format: schema 0 for format 0, schema 1 for format 1 (ADR 0018).
    pub snapshot_schema: SnapshotSchemaVersion,
    pub semantics_version: SemanticsVersion,
    pub execution_id: ExecutionId,
    pub program_artifact_id: ProgramArtifactId,
    pub turn: Turn,
    pub interaction_counter: u64,
    pub globals: BTreeMap<GlobalId, Value>,
    /// Always present in schema 0. In schema 1, present exactly when the Program uses
    /// `ReconcileScene`.
    pub scene: Option<SceneState>,
    pub statechart: Option<StatechartStateV0>,
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
    AwaitingEffect {
        vm: VmStateV0,
        pending: PendingEffectV0,
    },
    StatechartStable,
    AwaitingStatechartEffect {
        pending: PendingEffectV0,
    },
    StatechartFinished,
    Finished {
        result: Value,
        final_frames: Vec<FrameStateV0>,
    },
}

impl RuntimeStateV0 {
    pub fn pending(&self) -> Option<&PendingInteractionV0> {
        match &self.status {
            RuntimeStatusV0::Awaiting { pending, .. } => Some(pending),
            RuntimeStatusV0::Ready { .. }
            | RuntimeStatusV0::AwaitingEffect { .. }
            | RuntimeStatusV0::StatechartStable
            | RuntimeStatusV0::AwaitingStatechartEffect { .. }
            | RuntimeStatusV0::StatechartFinished
            | RuntimeStatusV0::Finished { .. } => None,
        }
    }

    pub fn vm(&self) -> Option<&VmStateV0> {
        match &self.status {
            RuntimeStatusV0::Ready { vm }
            | RuntimeStatusV0::Awaiting { vm, .. }
            | RuntimeStatusV0::AwaitingEffect { vm, .. } => Some(vm),
            RuntimeStatusV0::Finished { .. }
            | RuntimeStatusV0::StatechartStable
            | RuntimeStatusV0::AwaitingStatechartEffect { .. }
            | RuntimeStatusV0::StatechartFinished => None,
        }
    }

    pub fn pending_effect(&self) -> Option<&super::PendingEffectV0> {
        match &self.status {
            RuntimeStatusV0::AwaitingEffect { pending, .. } => Some(pending),
            RuntimeStatusV0::AwaitingStatechartEffect { pending } => Some(pending),
            RuntimeStatusV0::Ready { .. }
            | RuntimeStatusV0::Awaiting { .. }
            | RuntimeStatusV0::Finished { .. }
            | RuntimeStatusV0::StatechartStable
            | RuntimeStatusV0::StatechartFinished => None,
        }
    }
}
