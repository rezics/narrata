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
        BinaryOpV0, ChoiceArmV0, ConstIndex, ContentEntryV1, ContentIndex, ContentOperand,
        ContentRef, FlowV0, GlobalDeclV0, InstructionRecordV0, LocalDeclV0, OpV0,
        ProgramArtifactV0, ReturnModeV0, Segment, SlotRefV0, UnaryOpV0,
    },
    upgrade::{LocalContentPack, LocalTextEntry, UPGRADE_CONTENT_PROVIDER},
    version::{PROGRAM_FORMAT_V0, PROGRAM_FORMAT_V1, ProgramFormatVersion, SEMANTICS_V0},
};

/// Every generator builds one story in two formats. Format 0 (`*_v0`) keeps the text in
/// constants and is frozen with the Stage 1-5 corpora; format 1 (`*_v1`, ADR 0018) names
/// content-table entries whose text [`story_content`] holds.
const STORY_TEXTS: &[(&str, &str)] = &[
    ("choose-a-path", "Choose a path"),
    ("continue", "Continue"),
    ("effect-completed", "Effect completed"),
    ("hello", "Hello"),
    ("hidden", "Hidden"),
    ("inside-callee", "Inside callee"),
    ("narrator", "Narrator"),
    ("scene-ready", "Scene ready"),
    ("unexpected-branch", "Unexpected branch"),
];

/// The local content pack holding the text of every format 1 story.
pub fn story_content() -> LocalContentPack {
    let mut pack = LocalContentPack::new("en");
    for (key, text) in STORY_TEXTS {
        pack.entries.insert(
            (*key).to_owned(),
            LocalTextEntry {
                text: (*text).to_owned(),
            },
        );
    }
    pack
}

/// The reference a story names for `key`; `None` only for a key the local provider rejects.
pub fn story_reference(key: &str) -> Option<ContentRef> {
    ContentRef::new(UPGRADE_CONTENT_PROVIDER, key).ok()
}

pub fn scene_reconcile_v0() -> Result<ProgramArtifactV0, SceneError> {
    scene_reconcile(PROGRAM_FORMAT_V0)
}

pub fn scene_reconcile_v1() -> Result<ProgramArtifactV0, SceneError> {
    scene_reconcile(PROGRAM_FORMAT_V1)
}

fn scene_reconcile(format: ProgramFormatVersion) -> Result<ProgramArtifactV0, SceneError> {
    let legacy = format == PROGRAM_FORMAT_V0;
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
        format_version: format,
        semantics_version: SEMANTICS_V0,
        program_id: ProgramId::from_u128(40),
        entry_flow: flow,
        constants: if legacy {
            vec![Value::from("Scene ready")]
        } else {
            Vec::new()
        },
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
                        text: present(legacy, 0, 0),
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
        content: table(legacy, &[segment("scene-ready")]),
        statechart: None,
    })
}

pub fn recorded_query_v0() -> Result<ProgramArtifactV0, CapabilityIdError> {
    recorded_query(PROGRAM_FORMAT_V0)
}

pub fn recorded_query_v1() -> Result<ProgramArtifactV0, CapabilityIdError> {
    recorded_query(PROGRAM_FORMAT_V1)
}

fn recorded_query(format: ProgramFormatVersion) -> Result<ProgramArtifactV0, CapabilityIdError> {
    effect_program(
        "host.query",
        DeliveryPolicy::RecordedQuery,
        RewindPolicy::ReuseRecordedResponse,
        30,
        format,
    )
}

pub fn barrier_command_v0() -> Result<ProgramArtifactV0, CapabilityIdError> {
    barrier_command(PROGRAM_FORMAT_V0)
}

pub fn barrier_command_v1() -> Result<ProgramArtifactV0, CapabilityIdError> {
    barrier_command(PROGRAM_FORMAT_V1)
}

fn barrier_command(format: ProgramFormatVersion) -> Result<ProgramArtifactV0, CapabilityIdError> {
    effect_program(
        "host.command",
        DeliveryPolicy::AtLeastOnceIdempotent,
        RewindPolicy::Barrier,
        31,
        format,
    )
}

fn effect_program(
    capability: &str,
    delivery: DeliveryPolicy,
    rewind: RewindPolicy,
    program: u128,
    format: ProgramFormatVersion,
) -> Result<ProgramArtifactV0, CapabilityIdError> {
    let legacy = format == PROGRAM_FORMAT_V0;
    let flow = FlowId::from_u128(30);
    let result = GlobalId::from_u128(30);
    let capability = CapabilityId::new(capability)?;
    let version = CapabilityVersion::new(1).ok_or(CapabilityIdError::Invalid)?;
    Ok(ProgramArtifactV0 {
        format_version: format,
        semantics_version: SEMANTICS_V0,
        program_id: ProgramId::from_u128(program),
        entry_flow: flow,
        constants: if legacy {
            vec![Value::I64(7), Value::from("Effect completed")]
        } else {
            vec![Value::I64(7)]
        },
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
                        text: present(legacy, 1, 0),
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
        content: table(legacy, &[segment("effect-completed")]),
        statechart: None,
    })
}

pub fn hello_v0() -> ProgramArtifactV0 {
    hello(PROGRAM_FORMAT_V0)
}

pub fn hello_v1() -> ProgramArtifactV0 {
    hello(PROGRAM_FORMAT_V1)
}

fn hello(format: ProgramFormatVersion) -> ProgramArtifactV0 {
    let legacy = format == PROGRAM_FORMAT_V0;
    let flow = FlowId::from_u128(1);
    let say = InstructionId::from_u128(1);
    let finish = InstructionId::from_u128(2);
    ProgramArtifactV0 {
        format_version: format,
        semantics_version: SEMANTICS_V0,
        program_id: ProgramId::from_u128(1),
        entry_flow: flow,
        constants: if legacy {
            vec![Value::from("Hello")]
        } else {
            Vec::new()
        },
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
                        text: present(legacy, 0, 0),
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
        content: table(legacy, &[segment("hello")]),
        statechart: None,
    }
}

pub fn branch_call_choice_v0() -> ProgramArtifactV0 {
    branch_call_choice(PROGRAM_FORMAT_V0)
}

pub fn branch_call_choice_v1() -> ProgramArtifactV0 {
    branch_call_choice(PROGRAM_FORMAT_V1)
}

/// Format 1 keeps the constants rules use (`"argument"` is a call argument, not text) and
/// renumbers them; presentation operands move to the content table.
fn branch_call_choice(format: ProgramFormatVersion) -> ProgramArtifactV0 {
    let legacy = format == PROGRAM_FORMAT_V0;
    let data =
        |legacy_index: u32, index: u32| ConstIndex(if legacy { legacy_index } else { index });
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
        format_version: format,
        semantics_version: SEMANTICS_V0,
        program_id: ProgramId::from_u128(2),
        entry_flow: entry,
        constants: if legacy {
            vec![
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
            ]
        } else {
            vec![
                Value::I64(2),
                Value::Bool(false),
                Value::from("argument"),
                Value::I64(41),
                Value::I64(1),
            ]
        },
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
                            constant: data(0, 0),
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
                            constant: data(1, 1),
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
                            constant: data(0, 0),
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
                            text: present(legacy, 2, 0),
                            next: instruction(12),
                        },
                    ),
                    record(
                        12,
                        OpV0::Choice {
                            prompt: Some(present(legacy, 2, 1)),
                            choices: vec![
                                ChoiceArmV0 {
                                    id: choice,
                                    label: present(legacy, 3, 2),
                                    visible_if: Some(SlotRefV0::Local(flag)),
                                    target: instruction(13),
                                },
                                ChoiceArmV0 {
                                    id: hidden_choice,
                                    label: present(legacy, 4, 3),
                                    visible_if: Some(SlotRefV0::Global(hidden)),
                                    target: instruction(90),
                                },
                            ],
                        },
                    ),
                    record(
                        13,
                        OpV0::Const {
                            constant: data(5, 2),
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
                            constant: data(9, 4),
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
                            text: present(legacy, 10, 4),
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
                            speaker: Some(present(legacy, 6, 5)),
                            text: present(legacy, 7, 6),
                            next: instruction(102),
                        },
                    ),
                    record(
                        102,
                        OpV0::Const {
                            constant: data(8, 3),
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
        content: table(
            legacy,
            &[
                segment("choose-a-path"),
                reference("choose-a-path"),
                reference("continue"),
                reference("hidden"),
                segment("unexpected-branch"),
                reference("narrator"),
                segment("inside-callee"),
            ],
        ),
        statechart: None,
    }
}

pub fn statechart_parallel_history_v0() -> Result<ProgramArtifactV0, CapabilityIdError> {
    statechart_parallel_history(PROGRAM_FORMAT_V0)
}

/// The Statechart story has no text; format 1 differs only in its format version.
pub fn statechart_parallel_history_v1() -> Result<ProgramArtifactV0, CapabilityIdError> {
    statechart_parallel_history(PROGRAM_FORMAT_V1)
}

fn statechart_parallel_history(
    format: ProgramFormatVersion,
) -> Result<ProgramArtifactV0, CapabilityIdError> {
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
        format_version: format,
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
        content: Vec::new(),
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

/// A format 0 presentation operand: the string constant at `index`.
fn legacy_text(index: u32) -> ContentOperand {
    ContentOperand::Constant(ConstIndex(index))
}

/// The operand of one presentation in either format: a string constant in format 0, a
/// content-table entry in format 1.
fn present(legacy: bool, constant: u32, entry: u32) -> ContentOperand {
    if legacy {
        legacy_text(constant)
    } else {
        ContentOperand::Content(ContentIndex(entry))
    }
}

/// Format 0 has no content table. Story keys are valid literals; a rejected one would drop
/// its entry and fail Program validation.
fn table(legacy: bool, entries: &[Option<ContentEntryV1>]) -> Vec<ContentEntryV1> {
    if legacy {
        Vec::new()
    } else {
        entries.iter().flatten().cloned().collect()
    }
}

fn reference(key: &str) -> Option<ContentEntryV1> {
    story_reference(key).map(ContentEntryV1::Ref)
}

fn segment(key: &str) -> Option<ContentEntryV1> {
    story_reference(key).map(|unit| ContentEntryV1::Segment(Segment::unit(unit)))
}

fn record(id: u128, op: OpV0) -> InstructionRecordV0 {
    InstructionRecordV0 {
        id: InstructionId::from_u128(id),
        op,
    }
}
