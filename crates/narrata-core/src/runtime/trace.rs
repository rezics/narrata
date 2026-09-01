use crate::identity::{FlowId, InstructionId, ReceiptDigest, StateDigest};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceEventV0 {
    pub turn: u64,
    pub flow: FlowId,
    pub instruction: InstructionId,
    pub opcode: &'static str,
    pub frame_depth: usize,
    pub stack_depth: usize,
    pub status: &'static str,
    pub state_digest: Option<StateDigest>,
    pub receipt_digest: Option<ReceiptDigest>,
}
