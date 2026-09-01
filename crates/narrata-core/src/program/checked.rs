use std::collections::BTreeMap;

use crate::{
    identity::{FlowId, GlobalId, InstructionId, ProgramArtifactId},
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
}
