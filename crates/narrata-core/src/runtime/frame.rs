use std::collections::BTreeMap;

use crate::{
    identity::{FlowId, InstructionId, LocalId},
    value::Value,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmStateV0 {
    pub frames: Vec<FrameStateV0>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameStateV0 {
    pub flow: FlowId,
    pub instruction: InstructionId,
    pub return_to: Option<InstructionId>,
    pub locals: BTreeMap<LocalId, Value>,
    pub evaluation_stack: Vec<Value>,
}
