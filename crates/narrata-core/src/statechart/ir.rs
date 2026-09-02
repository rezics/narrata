use crate::{
    ActionId, CapabilityId, EventTypeId, FlowId, GlobalId, HistoryId, RegionId, StateId,
    TransitionId, Value, diagnostic::SourceSpan,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StateKindV0 {
    Atomic = 0,
    Compound = 1,
    Parallel = 2,
    Final = 3,
}

impl StateKindV0 {
    pub(crate) fn from_u64(value: u64) -> Option<Self> {
        match value {
            0 => Some(Self::Atomic),
            1 => Some(Self::Compound),
            2 => Some(Self::Parallel),
            3 => Some(Self::Final),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum HistoryKindV0 {
    Shallow = 0,
    Deep = 1,
}

impl HistoryKindV0 {
    pub(crate) fn from_u64(value: u64) -> Option<Self> {
        match value {
            0 => Some(Self::Shallow),
            1 => Some(Self::Deep),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GuardExprV0 {
    Always,
    Bool { global: GlobalId, expected: bool },
    Equals { global: GlobalId, value: Value },
    Not(Box<GuardExprV0>),
    All(Vec<GuardExprV0>),
    Any(Vec<GuardExprV0>),
}

impl GuardExprV0 {
    pub fn always() -> Self {
        Self::Always
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StatechartActionV0 {
    Assign {
        global: GlobalId,
        value: Value,
    },
    Raise {
        event: EventTypeId,
    },
    StartFlow {
        flow: FlowId,
        result_to: Option<GlobalId>,
        done_event: Option<EventTypeId>,
    },
    EmitEffect {
        capability: CapabilityId,
        payload: Value,
        response_to: Option<GlobalId>,
        response_event: Option<EventTypeId>,
    },
}

impl StatechartActionV0 {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Assign { .. } => "assign",
            Self::Raise { .. } => "raise",
            Self::StartFlow { .. } => "start-flow",
            Self::EmitEffect { .. } => "emit-effect",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionRecordV0 {
    pub id: ActionId,
    pub action: StatechartActionV0,
    pub source_span: Option<SourceSpan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateV0 {
    pub id: StateId,
    pub parent: Option<StateId>,
    pub region: Option<RegionId>,
    pub kind: StateKindV0,
    pub on_entry: Vec<ActionRecordV0>,
    pub on_exit: Vec<ActionRecordV0>,
    pub completion_event: Option<EventTypeId>,
    pub source_span: Option<SourceSpan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegionV0 {
    pub id: RegionId,
    pub parent: StateId,
    pub initial: StateId,
    pub source_span: Option<SourceSpan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryV0 {
    pub id: HistoryId,
    pub parent: StateId,
    pub region: RegionId,
    pub kind: HistoryKindV0,
    pub default_targets: Vec<StateId>,
    pub source_span: Option<SourceSpan>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TransitionKindV0 {
    External = 0,
    Internal = 1,
}

impl TransitionKindV0 {
    pub(crate) fn from_u64(value: u64) -> Option<Self> {
        match value {
            0 => Some(Self::External),
            1 => Some(Self::Internal),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransitionTargetV0 {
    State(StateId),
    History(HistoryId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionV0 {
    pub id: TransitionId,
    pub source: StateId,
    pub event: Option<EventTypeId>,
    pub kind: TransitionKindV0,
    pub guard: GuardExprV0,
    pub targets: Vec<TransitionTargetV0>,
    pub actions: Vec<ActionRecordV0>,
    pub source_span: Option<SourceSpan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatechartV0 {
    pub root: StateId,
    pub events: Vec<EventTypeId>,
    pub states: Vec<StateV0>,
    pub regions: Vec<RegionV0>,
    pub histories: Vec<HistoryV0>,
    pub transitions: Vec<TransitionV0>,
    pub source_span: Option<SourceSpan>,
}
