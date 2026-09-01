use std::collections::BTreeMap;

use narrata_core::{
    ActorId, ActorState, AudioChannelId, AudioState, CameraState, CapabilityDeclV0, CapabilityId,
    CapabilityIdError, CapabilityRequirement, CapabilityVersion, ChoiceId, DeliveryPolicy,
    EntityId, FlowId, GlobalId, InstructionId, LayerId, LayerState, LocalId, ProgramId,
    RewindPolicy, SceneError, SceneState, Value, ValueKindV0, ValueSchemaV0,
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
    }
}

fn record(id: u128, op: OpV0) -> InstructionRecordV0 {
    InstructionRecordV0 {
        id: InstructionId::from_u128(id),
        op,
    }
}
