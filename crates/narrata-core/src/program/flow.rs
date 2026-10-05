use crate::{
    identity::{FlowId, GlobalId, InstructionId, LocalId},
    value::{Value, ValueKindV0},
};

use super::InstructionRecordV0;
pub use crate::effect::CapabilityDeclV0;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalDeclV0 {
    pub id: GlobalId,
    pub kind: ValueKindV0,
    pub default: Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalDeclV0 {
    pub id: LocalId,
    pub kind: ValueKindV0,
    pub default: Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlowV0 {
    pub id: FlowId,
    pub parameters: Vec<LocalDeclV0>,
    pub locals: Vec<LocalDeclV0>,
    pub return_kind: Option<ValueKindV0>,
    pub entry: InstructionId,
    pub instructions: Vec<InstructionRecordV0>,
}
