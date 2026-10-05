use crate::{
    effect::CapabilityId,
    identity::{ChoiceId, EventTypeId, FlowId, GlobalId, InstructionId, LocalId},
    scene::SceneState,
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ConstIndex(pub u32);

/// An index into the Program content table (format 1).
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ContentIndex(pub u32);

/// What a presentation operand (`Say` speaker and text, `Choice` prompt and labels) names.
/// Both encode as the same unsigned integer; the Program format decides which one it is.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ContentOperand {
    /// Program format 0: a string constant holding the reader text.
    Constant(ConstIndex),
    /// Program format 1: a content-table entry the host resolves.
    Content(ContentIndex),
}

impl ContentOperand {
    pub(crate) const fn raw(self) -> u32 {
        match self {
            Self::Constant(ConstIndex(index)) | Self::Content(ContentIndex(index)) => index,
        }
    }
}

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
    pub label: ContentOperand,
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
        speaker: Option<ContentOperand>,
        text: ContentOperand,
        next: InstructionId,
    },
    Choice {
        prompt: Option<ContentOperand>,
        choices: Vec<ChoiceArmV0>,
    },
    Effect {
        capability: CapabilityId,
        next: InstructionId,
    },
    ReconcileScene {
        target: SceneState,
        next: InstructionId,
    },
    Raise {
        event: EventTypeId,
        next: InstructionId,
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
            Self::Effect { .. } => "Effect",
            Self::ReconcileScene { .. } => "ReconcileScene",
            Self::Raise { .. } => "Raise",
            Self::Finish { .. } => "Finish",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstructionRecordV0 {
    pub id: InstructionId,
    pub op: OpV0,
}
