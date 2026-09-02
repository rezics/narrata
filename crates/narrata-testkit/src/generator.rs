use std::collections::BTreeMap;

use narrata_core::{
    ActionId, ActionRecordV0, ActorId, ActorState, AudioChannelId, AudioState, CameraState,
    CapabilityDeclV0, CapabilityId, CapabilityIdError, CapabilityRequirement, CapabilityVersion,
    ChoiceId, DeliveryPolicy, EntityId, EventTypeId, FlowId, GlobalId, GuardExprV0, HistoryId,
    HistoryKindV0, HistoryV0, InstructionId, LayerId, LayerState, LocalId, ProgramId, RegionId,
    RegionV0, RewindPolicy, SceneError, SceneState, StateId, StateKindV0, StateV0,
    StatechartActionV0, StatechartV0, TransitionId, TransitionKindV0, TransitionTargetV0,
    TransitionV0, Value, ValueKindV0, ValueSchemaV0,
    program::{
        BinaryOpV0, ChoiceArmV0, ConstIndex, FlowV0, GlobalDeclV0, InstructionRecordV0,
        LocalDeclV0, OpV0, ProgramArtifactV0, ReturnModeV0, SlotRefV0, UnaryOpV0,
    },
    version::{PROGRAM_FORMAT_V0, SEMANTICS_V0},
};

pub fn scene_reconcile_v0() -> Result<ProgramArtifactV0, SceneError> {
    let flow = FlowId::from_u128(40);
    let layer = LayerId::from_u128(1);
    let actor = ActorId::from_u128(1);
    let scene = SceneState::checked(
        vec![LayerState {
            id: layer,
            asset: Some(EntityId::from_u128(1)),
            visible: true,
            z_index: 0,
        }],
        [(
            actor,
            ActorState {
                id: actor,
                asset: EntityId::from_u128(2),
                layer,
                visible: true,
                x_milli: 250,
                y_milli: -100,
            },
        )]
        .into_iter()
        .collect::<BTreeMap<_, _>>(),
        CameraState {
            x_milli: 5,
            y_milli: 10,
            zoom_milli: 1_250,
        },
        [(
            AudioChannelId::from_u128(1),
            AudioState {
                asset: EntityId::from_u128(3),
                playing: true,
                looping: true,
                position_millis: Some(500),
            },
        )]
        .into_iter()
        .collect::<BTreeMap<_, _>>(),
        None,
    )?;
    Ok(ProgramArtifactV0 {
        format_version: PROGRAM_FORMAT_V0,
        semantics_version: SEMANTICS_V0,
        program_id: ProgramId::from_u128(40),
        entry_flow: flow,
        constants: vec![Value::from("Scene ready")],
        globals: Vec::new(),
        flows: vec![FlowV0 {
            id: flow,
            parameters: Vec::new(),
            locals: Vec::new(),
            return_kind: None,
            entry: InstructionId::from_u128(1),
            instructions: vec![
                record(
                    1,
                    OpV0::ReconcileScene {
                        target: scene,
                        next: InstructionId::from_u128(2),
                    },
                ),
                record(
                    2,
                    OpV0::Say {
                        speaker: None,
                        text: ConstIndex(0),
                        next: InstructionId::from_u128(3),
                    },
                ),
                record(
                    3,
                    OpV0::Finish {
                        value: ReturnModeV0::None,
                    },
                ),
            ],
        }],
        capabilities: Vec::new(),
        external_content: Vec::new(),
        statechart: None,
    })
}

pub fn recorded_query_v0() -> Result<ProgramArtifactV0, CapabilityIdError> {
    effect_v0(
        "host.query",
        DeliveryPolicy::RecordedQuery,
        RewindPolicy::ReuseRecordedResponse,
        30,
    )
}

pub fn barrier_command_v0() -> Result<ProgramArtifactV0, CapabilityIdError> {
    effect_v0(
        "host.command",
        DeliveryPolicy::AtLeastOnceIdempotent,
        RewindPolicy::Barrier,
        31,
    )
}

fn effect_v0(
    capability: &str,
    delivery: DeliveryPolicy,
    rewind: RewindPolicy,
    program: u128,
) -> Result<ProgramArtifactV0, CapabilityIdError> {
    let flow = FlowId::from_u128(30);
    let result = GlobalId::from_u128(30);
    let capability = CapabilityId::new(capability)?;
    let version = CapabilityVersion::new(1).ok_or(CapabilityIdError::Invalid)?;
    Ok(ProgramArtifactV0 {
        format_version: PROGRAM_FORMAT_V0,
        semantics_version: SEMANTICS_V0,
        program_id: ProgramId::from_u128(program),
        entry_flow: flow,
        constants: vec![Value::I64(7), Value::from("Effect completed")],
        globals: vec![GlobalDeclV0 {
            id: result,
            kind: ValueKindV0::I64,
            default: Value::I64(0),
        }],
        flows: vec![FlowV0 {
            id: flow,
            parameters: Vec::new(),
            locals: Vec::new(),
            return_kind: None,
            entry: InstructionId::from_u128(1),
            instructions: vec![
                record(
                    1,
                    OpV0::Const {
                        constant: ConstIndex(0),
                        next: InstructionId::from_u128(2),
                    },
                ),
                record(
                    2,
                    OpV0::Effect {
                        capability: capability.clone(),
                        next: InstructionId::from_u128(3),
                    },
                ),
                record(
                    3,
                    OpV0::Store {
                        slot: SlotRefV0::Global(result),
                        next: InstructionId::from_u128(4),
                    },
                ),
                record(
                    4,
                    OpV0::Say {
                        speaker: None,
                        text: ConstIndex(1),
                        next: InstructionId::from_u128(5),
                    },
                ),
                record(
                    5,
                    OpV0::Finish {
                        value: ReturnModeV0::None,
                    },
                ),
            ],
        }],
        capabilities: vec![CapabilityDeclV0 {
            id: capability,
            version,
            requirement: CapabilityRequirement::Required,
            request_schema: ValueSchemaV0::I64,
            response_schema: ValueSchemaV0::I64,
            delivery,
            rewind,
        }],
        external_content: Vec::new(),
        statechart: None,
    })
}

pub fn hello_v0() -> ProgramArtifactV0 {
    let flow = FlowId::from_u128(1);
    let say = InstructionId::from_u128(1);
    let finish = InstructionId::from_u128(2);
    ProgramArtifactV0 {
        format_version: PROGRAM_FORMAT_V0,
        semantics_version: SEMANTICS_V0,
        program_id: ProgramId::from_u128(1),
        entry_flow: flow,
        constants: vec![Value::from("Hello")],
        globals: Vec::new(),
        flows: vec![FlowV0 {
            id: flow,
            parameters: Vec::new(),
            locals: Vec::new(),
            return_kind: None,
            entry: say,
            instructions: vec![
                InstructionRecordV0 {
                    id: say,
                    op: OpV0::Say {
                        speaker: None,
                        text: ConstIndex(0),
                        next: finish,
                    },
                },
                InstructionRecordV0 {
                    id: finish,
                    op: OpV0::Finish {
                        value: ReturnModeV0::None,
                    },
                },
            ],
        }],
        capabilities: Vec::new(),
        external_content: Vec::new(),
        statechart: None,
    }
}

pub fn branch_call_choice_v0() -> ProgramArtifactV0 {
    let entry = FlowId::from_u128(10);
    let callee = FlowId::from_u128(20);
    let score = GlobalId::from_u128(1);
    let hidden = GlobalId::from_u128(2);
    let flag = LocalId::from_u128(1);
    let parameter = LocalId::from_u128(2);
    let temporary = LocalId::from_u128(3);
    let instruction = |value| InstructionId::from_u128(value);
    let choice = ChoiceId::from_u128(1);
    let hidden_choice = ChoiceId::from_u128(2);
    ProgramArtifactV0 {
        format_version: PROGRAM_FORMAT_V0,
        semantics_version: SEMANTICS_V0,
        program_id: ProgramId::from_u128(2),
        entry_flow: entry,
        constants: vec![
            Value::I64(2),
            Value::Bool(false),
            Value::from("Choose a path"),
            Value::from("Continue"),
            Value::from("Hidden"),
            Value::from("argument"),
            Value::from("Narrator"),
            Value::from("Inside callee"),
            Value::I64(41),
            Value::I64(1),
            Value::from("Unexpected branch"),
        ],
        globals: vec![
            GlobalDeclV0 {
                id: score,
                kind: ValueKindV0::I64,
                default: Value::I64(0),
            },
            GlobalDeclV0 {
                id: hidden,
                kind: ValueKindV0::Bool,
                default: Value::Bool(false),
            },
        ],
        flows: vec![
            FlowV0 {
                id: entry,
                parameters: Vec::new(),
                locals: vec![LocalDeclV0 {
                    id: flag,
                    kind: ValueKindV0::Bool,
                    default: Value::Bool(false),
                }],
                return_kind: None,
                entry: instruction(1),
                instructions: vec![
                    record(
                        1,
                        OpV0::Const {
                            constant: ConstIndex(0),
                            next: instruction(2),
                        },
                    ),
                    record(
                        2,
                        OpV0::Store {
                            slot: SlotRefV0::Global(score),
                            next: instruction(3),
                        },
                    ),
                    record(
                        3,
                        OpV0::Const {
                            constant: ConstIndex(1),
                            next: instruction(4),
                        },
                    ),
                    record(
                        4,
                        OpV0::Unary {
                            op: UnaryOpV0::Not,
                            next: instruction(5),
                        },
                    ),
                    record(
                        5,
                        OpV0::Store {
                            slot: SlotRefV0::Local(flag),
                            next: instruction(6),
                        },
                    ),
                    record(
                        6,
                        OpV0::Load {
                            slot: SlotRefV0::Global(score),
                            next: instruction(7),
                        },
                    ),
                    record(
                        7,
                        OpV0::Const {
                            constant: ConstIndex(0),
                            next: instruction(8),
                        },
                    ),
                    record(
                        8,
                        OpV0::Binary {
                            op: BinaryOpV0::Eq,
                            next: instruction(9),
                        },
                    ),
                    record(
                        9,
                        OpV0::JumpIfFalse {
                            if_true: instruction(10),
                            if_false: instruction(90),
                        },
                    ),
                    record(
                        10,
                        OpV0::Jump {
                            target: instruction(11),
                        },
                    ),
                    record(
                        11,
                        OpV0::Say {
                            speaker: None,
                            text: ConstIndex(2),
                            next: instruction(12),
                        },
                    ),
                    record(
                        12,
                        OpV0::Choice {
                            prompt: Some(ConstIndex(2)),
                            choices: vec![
                                ChoiceArmV0 {
                                    id: choice,
                                    label: ConstIndex(3),
                                    visible_if: Some(SlotRefV0::Local(flag)),
                                    target: instruction(13),
                                },
                                ChoiceArmV0 {
                                    id: hidden_choice,
                                    label: ConstIndex(4),
                                    visible_if: Some(SlotRefV0::Global(hidden)),
                                    target: instruction(90),
                                },
                            ],
                        },
                    ),
                    record(
                        13,
                        OpV0::Const {
                            constant: ConstIndex(5),
                            next: instruction(14),
                        },
                    ),
                    record(
                        14,
                        OpV0::Call {
                            flow: callee,
                            argument_count: 1,
                            return_to: instruction(15),
                        },
                    ),
                    record(
                        15,
                        OpV0::Store {
                            slot: SlotRefV0::Global(score),
                            next: instruction(16),
                        },
                    ),
                    record(
                        16,
                        OpV0::Load {
                            slot: SlotRefV0::Global(score),
                            next: instruction(17),
                        },
                    ),
                    record(
                        17,
                        OpV0::Const {
                            constant: ConstIndex(9),
                            next: instruction(18),
                        },
                    ),
                    record(
                        18,
                        OpV0::Binary {
                            op: BinaryOpV0::Add,
                            next: instruction(19),
                        },
                    ),
                    record(
                        19,
                        OpV0::Store {
                            slot: SlotRefV0::Global(score),
                            next: instruction(20),
                        },
                    ),
                    record(
                        20,
                        OpV0::Finish {
                            value: ReturnModeV0::None,
                        },
                    ),
                    record(
                        90,
                        OpV0::Say {
                            speaker: None,
                            text: ConstIndex(10),
                            next: instruction(20),
                        },
                    ),
                ],
            },
            FlowV0 {
                id: callee,
                parameters: vec![LocalDeclV0 {
                    id: parameter,
                    kind: ValueKindV0::String,
                    default: Value::from(""),
                }],
                locals: vec![LocalDeclV0 {
                    id: temporary,
                    kind: ValueKindV0::I64,
                    default: Value::I64(0),
                }],
                return_kind: Some(ValueKindV0::I64),
                entry: instruction(101),
                instructions: vec![
                    record(
                        101,
                        OpV0::Say {
                            speaker: Some(ConstIndex(6)),
                            text: ConstIndex(7),
                            next: instruction(102),
                        },
                    ),
                    record(
                        102,
                        OpV0::Const {
                            constant: ConstIndex(8),
                            next: instruction(103),
                        },
                    ),
                    record(
                        103,
                        OpV0::Store {
                            slot: SlotRefV0::Local(temporary),
                            next: instruction(104),
                        },
                    ),
                    record(
                        104,
                        OpV0::Load {
                            slot: SlotRefV0::Local(temporary),
                            next: instruction(105),
                        },
                    ),
                    record(
                        105,
                        OpV0::Return {
                            value: ReturnModeV0::Stack,
                        },
                    ),
                ],
            },
        ],
        capabilities: Vec::new(),
        external_content: Vec::new(),
        statechart: None,
    }
}

pub fn statechart_parallel_history_v0() -> Result<ProgramArtifactV0, CapabilityIdError> {
    let entry = FlowId::from_u128(500);
    let invoked = FlowId::from_u128(501);
    let result = GlobalId::from_u128(500);
    let left_done = GlobalId::from_u128(501);
    let right_done = GlobalId::from_u128(502);
    let capability = CapabilityId::new("host.query")?;
    let capability_version = CapabilityVersion::new(1).ok_or(CapabilityIdError::Invalid)?;
    let event = |id| EventTypeId::from_u128(id);
    let state = |id| StateId::from_u128(id);
    let region = |id| RegionId::from_u128(id);
    let transition = |id| TransitionId::from_u128(id);
    let action_id = |id| ActionId::from_u128(id);
    let action_record = |id, value| ActionRecordV0 {
        id: action_id(id),
        action: value,
        source_span: None,
    };
    let state_record = |id: u128,
                        parent: Option<u128>,
                        region_id: Option<u128>,
                        kind: StateKindV0,
                        on_entry: Vec<ActionRecordV0>,
                        completion_event: Option<u128>| StateV0 {
        id: state(id),
        parent: parent.map(state),
        region: region_id.map(region),
        kind,
        on_entry,
        on_exit: Vec::new(),
        completion_event: completion_event.map(event),
        source_span: None,
    };
    let transition_record =
        |id: u128,
         source: u128,
         trigger: Option<u128>,
         targets: Vec<TransitionTargetV0>,
         actions: Vec<ActionRecordV0>| TransitionV0 {
            id: transition(id),
            source: state(source),
            event: trigger.map(event),
            kind: TransitionKindV0::External,
            guard: GuardExprV0::Always,
            targets,
            actions,
            source_span: None,
        };
    Ok(ProgramArtifactV0 {
        format_version: PROGRAM_FORMAT_V0,
        semantics_version: SEMANTICS_V0,
        program_id: ProgramId::from_u128(500),
        entry_flow: entry,
        constants: vec![Value::I64(7)],
        globals: vec![
            GlobalDeclV0 {
                id: result,
                kind: ValueKindV0::I64,
                default: Value::I64(0),
            },
            GlobalDeclV0 {
                id: left_done,
                kind: ValueKindV0::Bool,
                default: Value::Bool(false),
            },
            GlobalDeclV0 {
                id: right_done,
                kind: ValueKindV0::Bool,
                default: Value::Bool(false),
            },
        ],
        flows: vec![
            FlowV0 {
                id: entry,
                parameters: Vec::new(),
                locals: Vec::new(),
                return_kind: None,
                entry: InstructionId::from_u128(500),
                instructions: vec![record(
                    500,
                    OpV0::Finish {
                        value: ReturnModeV0::None,
                    },
                )],
            },
            FlowV0 {
                id: invoked,
                parameters: Vec::new(),
                locals: Vec::new(),
                return_kind: None,
                entry: InstructionId::from_u128(501),
                instructions: vec![
                    record(
                        501,
                        OpV0::Const {
                            constant: ConstIndex(0),
                            next: InstructionId::from_u128(502),
                        },
                    ),
                    record(
                        502,
                        OpV0::Effect {
                            capability: capability.clone(),
                            next: InstructionId::from_u128(503),
                        },
                    ),
                    record(
                        503,
                        OpV0::Store {
                            slot: SlotRefV0::Global(result),
                            next: InstructionId::from_u128(504),
                        },
                    ),
                    record(
                        504,
                        OpV0::Raise {
                            event: event(1),
                            next: InstructionId::from_u128(505),
                        },
                    ),
                    record(
                        505,
                        OpV0::Return {
                            value: ReturnModeV0::None,
                        },
                    ),
                ],
            },
        ],
        capabilities: vec![CapabilityDeclV0 {
            id: capability.clone(),
            version: capability_version,
            requirement: CapabilityRequirement::Required,
            request_schema: ValueSchemaV0::I64,
            response_schema: ValueSchemaV0::I64,
            delivery: DeliveryPolicy::RecordedQuery,
            rewind: RewindPolicy::ReuseRecordedResponse,
        }],
        external_content: Vec::new(),
        statechart: Some(StatechartV0 {
            root: state(1),
            events: (1..=6).map(event).collect(),
            states: vec![
                state_record(1, None, None, StateKindV0::Compound, Vec::new(), None),
                state_record(
                    2,
                    Some(1),
                    Some(10),
                    StateKindV0::Atomic,
                    vec![action_record(
                        1,
                        StatechartActionV0::StartFlow {
                            flow: invoked,
                            result_to: None,
                            done_event: None,
                        },
                    )],
                    None,
                ),
                state_record(
                    3,
                    Some(1),
                    Some(10),
                    StateKindV0::Atomic,
                    vec![action_record(
                        2,
                        StatechartActionV0::EmitEffect {
                            capability,
                            payload: Value::I64(9),
                            response_to: Some(result),
                            response_event: Some(event(2)),
                        },
                    )],
                    None,
                ),
                state_record(
                    4,
                    Some(1),
                    Some(10),
                    StateKindV0::Parallel,
                    Vec::new(),
                    Some(6),
                ),
                state_record(5, Some(1), Some(10), StateKindV0::Atomic, Vec::new(), None),
                state_record(6, Some(1), Some(10), StateKindV0::Final, Vec::new(), None),
                state_record(10, Some(4), Some(11), StateKindV0::Atomic, Vec::new(), None),
                state_record(11, Some(4), Some(11), StateKindV0::Final, Vec::new(), None),
                state_record(20, Some(4), Some(12), StateKindV0::Atomic, Vec::new(), None),
                state_record(21, Some(4), Some(12), StateKindV0::Final, Vec::new(), None),
            ],
            regions: vec![
                RegionV0 {
                    id: region(10),
                    parent: state(1),
                    initial: state(2),
                    source_span: None,
                },
                RegionV0 {
                    id: region(11),
                    parent: state(4),
                    initial: state(10),
                    source_span: None,
                },
                RegionV0 {
                    id: region(12),
                    parent: state(4),
                    initial: state(20),
                    source_span: None,
                },
            ],
            histories: vec![
                HistoryV0 {
                    id: HistoryId::from_u128(1),
                    parent: state(4),
                    region: region(11),
                    kind: HistoryKindV0::Deep,
                    default_targets: vec![state(10)],
                    source_span: None,
                },
                HistoryV0 {
                    id: HistoryId::from_u128(2),
                    parent: state(4),
                    region: region(12),
                    kind: HistoryKindV0::Shallow,
                    default_targets: vec![state(20)],
                    source_span: None,
                },
            ],
            transitions: vec![
                transition_record(
                    1,
                    2,
                    Some(1),
                    vec![TransitionTargetV0::State(state(3))],
                    Vec::new(),
                ),
                transition_record(
                    2,
                    3,
                    Some(2),
                    vec![TransitionTargetV0::State(state(4))],
                    Vec::new(),
                ),
                transition_record(
                    3,
                    4,
                    Some(3),
                    vec![TransitionTargetV0::State(state(5))],
                    Vec::new(),
                ),
                transition_record(
                    4,
                    5,
                    Some(4),
                    vec![
                        TransitionTargetV0::History(HistoryId::from_u128(1)),
                        TransitionTargetV0::History(HistoryId::from_u128(2)),
                    ],
                    Vec::new(),
                ),
                transition_record(
                    5,
                    10,
                    Some(5),
                    vec![TransitionTargetV0::State(state(11))],
                    vec![action_record(
                        3,
                        StatechartActionV0::Assign {
                            global: left_done,
                            value: Value::Bool(true),
                        },
                    )],
                ),
                transition_record(
                    6,
                    20,
                    Some(5),
                    vec![TransitionTargetV0::State(state(21))],
                    vec![action_record(
                        4,
                        StatechartActionV0::Assign {
                            global: right_done,
                            value: Value::Bool(true),
                        },
                    )],
                ),
                transition_record(
                    7,
                    4,
                    Some(6),
                    vec![TransitionTargetV0::State(state(6))],
                    Vec::new(),
                ),
            ],
            source_span: None,
        }),
    })
}

fn record(id: u128, op: OpV0) -> InstructionRecordV0 {
    InstructionRecordV0 {
        id: InstructionId::from_u128(id),
        op,
    }
}
