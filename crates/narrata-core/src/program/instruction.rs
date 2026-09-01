use crate::identity::{ChoiceId, FlowId, GlobalId, InstructionId, LocalId};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ConstIndex(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotRefV0 {
    Global(GlobalId),
    Local(LocalId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum UnaryOpV0 {
    Not = 0,
    Neg = 1,
}

impl UnaryOpV0 {
    pub(crate) fn from_u64(value: u64) -> Option<Self> {
        match value {
            0 => Some(Self::Not),
            1 => Some(Self::Neg),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum BinaryOpV0 {
    Eq = 0,
    Ne = 1,
    Add = 2,
    Sub = 3,
    Mul = 4,
    Div = 5,
    Rem = 6,
    Lt = 7,
    Le = 8,
    Gt = 9,
    Ge = 10,
}

impl BinaryOpV0 {
    pub(crate) fn from_u64(value: u64) -> Option<Self> {
        match value {
            0 => Some(Self::Eq),
            1 => Some(Self::Ne),
            2 => Some(Self::Add),
            3 => Some(Self::Sub),
            4 => Some(Self::Mul),
            5 => Some(Self::Div),
            6 => Some(Self::Rem),
            7 => Some(Self::Lt),
            8 => Some(Self::Le),
            9 => Some(Self::Gt),
            10 => Some(Self::Ge),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ReturnModeV0 {
    None = 0,
    Stack = 1,
}

impl ReturnModeV0 {
    pub(crate) fn from_u64(value: u64) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::Stack),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChoiceArmV0 {
    pub id: ChoiceId,
    pub label: ConstIndex,
    pub visible_if: Option<SlotRefV0>,
    pub target: InstructionId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpV0 {
    Const {
        constant: ConstIndex,
        next: InstructionId,
    },
    Load {
        slot: SlotRefV0,
        next: InstructionId,
    },
    Store {
        slot: SlotRefV0,
        next: InstructionId,
    },
    Unary {
        op: UnaryOpV0,
        next: InstructionId,
    },
    Binary {
        op: BinaryOpV0,
        next: InstructionId,
    },
    Jump {
        target: InstructionId,
    },
    JumpIfFalse {
        if_true: InstructionId,
        if_false: InstructionId,
    },
    Call {
        flow: FlowId,
        argument_count: u16,
        return_to: InstructionId,
    },
    Return {
        value: ReturnModeV0,
    },
    Say {
        speaker: Option<ConstIndex>,
        text: ConstIndex,
        next: InstructionId,
    },
    Choice {
        prompt: Option<ConstIndex>,
        choices: Vec<ChoiceArmV0>,
    },
    Finish {
        value: ReturnModeV0,
    },
}

impl OpV0 {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Const { .. } => "Const",
            Self::Load { .. } => "Load",
            Self::Store { .. } => "Store",
            Self::Unary { .. } => "Unary",
            Self::Binary { .. } => "Binary",
            Self::Jump { .. } => "Jump",
            Self::JumpIfFalse { .. } => "JumpIfFalse",
            Self::Call { .. } => "Call",
            Self::Return { .. } => "Return",
            Self::Say { .. } => "Say",
            Self::Choice { .. } => "Choice",
            Self::Finish { .. } => "Finish",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstructionRecordV0 {
    pub id: InstructionId,
    pub op: OpV0,
}
