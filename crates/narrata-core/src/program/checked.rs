use std::collections::BTreeMap;

use crate::{
    effect::{CapabilityDeclV0, CapabilityId},
    identity::{FlowId, GlobalId, InstructionId, ProgramArtifactId},
    identity::{HistoryId, RegionId, StateId, TransitionId},
    statechart::{
        ActionRecordV0, HistoryV0, RegionV0, StateV0, StatechartIndices, StatechartV0, TransitionV0,
    },
    value::{Value, ValueKindV0},
};

use super::{FlowV0, GlobalDeclV0, InstructionRecordV0, ProgramArtifactV0};

#[derive(Clone, Debug)]
pub struct CheckedProgram {
    pub(crate) artifact: ProgramArtifactV0,
    pub(crate) artifact_id: ProgramArtifactId,
    pub(crate) flow_indices: BTreeMap<FlowId, usize>,
    pub(crate) instruction_indices: BTreeMap<FlowId, BTreeMap<InstructionId, usize>>,
    pub(crate) global_indices: BTreeMap<GlobalId, usize>,
    pub(crate) stack_limits: BTreeMap<FlowId, usize>,
    pub(crate) statechart_indices: StatechartIndices,
}

impl CheckedProgram {
    pub fn artifact_id(&self) -> ProgramArtifactId {
        self.artifact_id
    }

    pub fn artifact(&self) -> &ProgramArtifactV0 {
        &self.artifact
    }

    pub fn flow(&self, id: FlowId) -> Option<&FlowV0> {
        self.flow_indices
            .get(&id)
            .and_then(|index| self.artifact.flows.get(*index))
    }

    pub fn instruction(
        &self,
        flow: FlowId,
        instruction: InstructionId,
    ) -> Option<&InstructionRecordV0> {
        let index = self
            .instruction_indices
            .get(&flow)
            .and_then(|indices| indices.get(&instruction))?;
        self.flow(flow)?.instructions.get(*index)
    }

    pub fn constant(&self, index: super::ConstIndex) -> Option<&Value> {
        self.artifact.constants.get(index.0 as usize)
    }

    pub fn global(&self, id: GlobalId) -> Option<&GlobalDeclV0> {
        self.global_indices
            .get(&id)
            .and_then(|index| self.artifact.globals.get(*index))
    }

    pub fn slot_kind(&self, flow: FlowId, slot: super::SlotRefV0) -> Option<ValueKindV0> {
        match slot {
            super::SlotRefV0::Global(id) => self.global(id).map(|decl| decl.kind),
            super::SlotRefV0::Local(id) => self.flow(flow).and_then(|flow| {
                flow.parameters
                    .iter()
                    .chain(&flow.locals)
                    .find(|decl| decl.id == id)
                    .map(|decl| decl.kind)
            }),
        }
    }

    pub fn maximum_stack_depth(&self, flow: FlowId) -> Option<usize> {
        self.stack_limits.get(&flow).copied()
    }

    pub fn capability(&self, id: &CapabilityId) -> Option<&CapabilityDeclV0> {
        self.artifact
            .capabilities
            .iter()
            .find(|capability| &capability.id == id)
    }

    pub fn statechart(&self) -> Option<&StatechartV0> {
        self.artifact.statechart.as_ref()
    }

    pub fn state(&self, id: StateId) -> Option<&StateV0> {
        let index = self.statechart_indices.states.get(&id)?;
        self.statechart()?.states.get(*index)
    }

    pub fn region(&self, id: RegionId) -> Option<&RegionV0> {
        let index = self.statechart_indices.regions.get(&id)?;
        self.statechart()?.regions.get(*index)
    }

    pub fn history(&self, id: HistoryId) -> Option<&HistoryV0> {
        let index = self.statechart_indices.histories.get(&id)?;
        self.statechart()?.histories.get(*index)
    }

    pub fn transition(&self, id: TransitionId) -> Option<&TransitionV0> {
        let index = self.statechart_indices.transitions.get(&id)?;
        self.statechart()?.transitions.get(*index)
    }

    pub fn statechart_action(&self, id: crate::ActionId) -> Option<&ActionRecordV0> {
        let chart = self.statechart()?;
        chart
            .states
            .iter()
            .flat_map(|state| state.on_entry.iter().chain(&state.on_exit))
            .chain(
                chart
                    .transitions
                    .iter()
                    .flat_map(|transition| transition.actions.iter()),
            )
            .find(|action| action.id == id)
    }

    pub(crate) fn statechart_indices(&self) -> &StatechartIndices {
        &self.statechart_indices
    }
}
