#![allow(clippy::panic, clippy::unwrap_used)]

use std::{collections::BTreeMap, sync::Arc};

use narrata_core::{
    CommitId, ExecutionId, FlowId, InputId, InstructionId, LocalId, MigrationId,
    RecoveryCheckpointId,
    migration::{
        DeclarativeMigration, InstructionLocation, LocalLocation, MigrationDescriptor,
        MigrationError, MigrationOptions, RecoveryPoint, RelocationTable, VersionRange,
        migrate_checked_state,
    },
    program::{OpV0, encode_program_artifact, load_program},
    runtime::{
        CheckedRuntimeInput, PendingInteractionV0, SliceBudget, SliceOutcome, begin_transition,
        begin_transition_with_parent_commit,
    },
};
use narrata_store::{
    BranchId, CommitCauseV1, FaultPoint, InitialRecordingMode, MemoryStore, MigrationRegistry,
    ProgramRegistry, RefKey, RefName, SaveStore, SessionCoordinator, apply_migration, load_commit,
};
use narrata_testkit::generator::branch_call_choice_v0;
use narrata_testkit::generator::hello_v0;
use narrata_testkit::generator::recorded_query_v0;
use narrata_testkit::generator::statechart_parallel_history_v0;

fn programs() -> (
    Arc<narrata_core::CheckedProgram>,
    Arc<narrata_core::CheckedProgram>,
    Arc<DeclarativeMigration>,
) {
    let source = load_program(&encode_program_artifact(&hello_v0()), &Default::default()).unwrap();
    let mut target_artifact = hello_v0();
    let flow = &mut target_artifact.flows[0];
    flow.entry = InstructionId::from_u128(11);
    flow.instructions[0].id = InstructionId::from_u128(11);
    if let OpV0::Say { next, .. } = &mut flow.instructions[0].op {
        *next = InstructionId::from_u128(12);
    }
    flow.instructions[1].id = InstructionId::from_u128(12);
    target_artifact.constants[0] = narrata_core::Value::from("Hello after migration");
    let target = load_program(
        &encode_program_artifact(&target_artifact),
        &Default::default(),
    )
    .unwrap();
    let migration = Arc::new(
        DeclarativeMigration::new(MigrationDescriptor {
            id: MigrationId::from_u128(1),
            from: source.artifact_id(),
            to: target.artifact_id(),
            accepted_snapshot_schemas: VersionRange {
                minimum: 0,
                maximum: 0,
            },
            relocations: RelocationTable {
                instructions: BTreeMap::from([
                    (InstructionId::from_u128(1), InstructionId::from_u128(11)),
                    (InstructionId::from_u128(2), InstructionId::from_u128(12)),
                ]),
                ..RelocationTable::default()
            },
            recovery_points: BTreeMap::new(),
            recovery_locations: BTreeMap::new(),
        })
        .unwrap(),
    );
    (source, target, migration)
}

fn source_store(
    source: Arc<narrata_core::CheckedProgram>,
) -> (MemoryStore, narrata_core::CommitId) {
    let mut coordinator = SessionCoordinator::create(
        MemoryStore::new(),
        source,
        ExecutionId::from_u128(1),
        RefName::new("session").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Standard,
        1,
    )
    .unwrap();
    let result = coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap();
    (coordinator.into_store(), result.commit)
}

#[test]
fn migration_creates_child_commit_preserves_source_and_continues() {
    let (source, target, migration) = programs();
    let (mut store, source_commit) = source_store(source.clone());
    let source_objects = store.list_objects().unwrap();
    let mut registry = MigrationRegistry::new();
    registry.register(migration).unwrap();
    let mut programs = ProgramRegistry::new();
    assert!(programs.register(source));
    assert!(programs.register(target.clone()));
    let migrated_ref = RefKey::save(
        RefName::new("player").unwrap(),
        RefName::new("migrated").unwrap(),
    );
    let applied = apply_migration(
        &mut store,
        &registry,
        &programs,
        source_commit,
        target.artifact_id(),
        None,
        MigrationOptions::default(),
        migrated_ref.clone(),
        None,
        3,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        store.read_ref(&migrated_ref).unwrap().unwrap().commit,
        applied.commit
    );
    assert!(
        source_objects
            .iter()
            .all(|object| store.get_object(object.id()).unwrap().is_some())
    );
    let loaded = load_commit(&store, applied.commit, &target).unwrap();
    assert_eq!(loaded.commit.parent, Some(source_commit));
    assert_eq!(
        loaded.commit.cause,
        CommitCauseV1::Migration(MigrationId::from_u128(1))
    );
    assert!(matches!(
        loaded.state.pending(),
        Some(narrata_core::runtime::PendingInteractionV0::Say { text, .. })
            if text.as_ref() == "Hello after migration"
    ));
}

#[test]
fn migration_fault_does_not_publish_objects_or_ref() {
    let (source, target, migration) = programs();
    let (mut store, source_commit) = source_store(source.clone());
    let before = store.list_objects().unwrap();
    let mut registry = MigrationRegistry::new();
    registry.register(migration).unwrap();
    let mut programs = ProgramRegistry::new();
    programs.register(source);
    programs.register(target.clone());
    let target_ref = RefKey::save(
        RefName::new("player").unwrap(),
        RefName::new("faulted").unwrap(),
    );
    store.inject_fault(Some(FaultPoint::RefCas));
    assert!(
        apply_migration(
            &mut store,
            &registry,
            &programs,
            source_commit,
            target.artifact_id(),
            None,
            MigrationOptions::default(),
            target_ref.clone(),
            None,
            3,
            &Default::default(),
        )
        .is_err()
    );
    assert_eq!(store.list_objects().unwrap(), before);
    assert!(store.read_ref(&target_ref).unwrap().is_none());
}

#[test]
fn automatic_path_selection_rejects_ambiguity() {
    let (source, target, first) = programs();
    let mut registry = MigrationRegistry::new();
    registry.register(first).unwrap();
    let second = DeclarativeMigration::new(MigrationDescriptor {
        id: MigrationId::from_u128(2),
        from: source.artifact_id(),
        to: target.artifact_id(),
        accepted_snapshot_schemas: VersionRange {
            minimum: 0,
            maximum: 0,
        },
        relocations: RelocationTable::default(),
        recovery_points: BTreeMap::new(),
        recovery_locations: BTreeMap::new(),
    })
    .unwrap();
    registry.register(Arc::new(second)).unwrap();
    assert!(matches!(
        registry.inspect(source.artifact_id(), target.artifact_id(), None),
        Err(narrata_store::MigrationRegistryError::Ambiguous { .. })
    ));
}

#[test]
fn flow_scoped_instruction_and_local_relocations_do_not_cross_wires() {
    let mut source_artifact = branch_call_choice_v0();
    let callee = &mut source_artifact.flows[1];
    callee.entry = InstructionId::from_u128(1);
    callee.locals[0].id = LocalId::from_u128(1);
    callee.instructions[0].id = InstructionId::from_u128(1);
    callee.instructions[0].op = OpV0::Say {
        speaker: Some(narrata_core::program::ConstIndex(6)),
        text: narrata_core::program::ConstIndex(7),
        next: InstructionId::from_u128(2),
    };
    callee.instructions[1].id = InstructionId::from_u128(2);
    callee.instructions[1].op = OpV0::Const {
        constant: narrata_core::program::ConstIndex(8),
        next: InstructionId::from_u128(3),
    };
    callee.instructions[2].id = InstructionId::from_u128(3);
    callee.instructions[2].op = OpV0::Store {
        slot: narrata_core::program::SlotRefV0::Local(LocalId::from_u128(1)),
        next: InstructionId::from_u128(4),
    };
    callee.instructions[3].id = InstructionId::from_u128(4);
    callee.instructions[3].op = OpV0::Load {
        slot: narrata_core::program::SlotRefV0::Local(LocalId::from_u128(1)),
        next: InstructionId::from_u128(5),
    };
    callee.instructions[4].id = InstructionId::from_u128(5);
    let source = load_program(
        &encode_program_artifact(&source_artifact),
        &Default::default(),
    )
    .unwrap();

    let mut target_artifact = source_artifact;
    let callee = &mut target_artifact.flows[1];
    callee.entry = InstructionId::from_u128(201);
    callee.locals[0].id = LocalId::from_u128(10);
    callee.instructions[0].id = InstructionId::from_u128(201);
    callee.instructions[0].op = OpV0::Say {
        speaker: Some(narrata_core::program::ConstIndex(6)),
        text: narrata_core::program::ConstIndex(7),
        next: InstructionId::from_u128(202),
    };
    callee.instructions[1].id = InstructionId::from_u128(202);
    callee.instructions[1].op = OpV0::Const {
        constant: narrata_core::program::ConstIndex(8),
        next: InstructionId::from_u128(203),
    };
    callee.instructions[2].id = InstructionId::from_u128(203);
    callee.instructions[2].op = OpV0::Store {
        slot: narrata_core::program::SlotRefV0::Local(LocalId::from_u128(10)),
        next: InstructionId::from_u128(204),
    };
    callee.instructions[3].id = InstructionId::from_u128(204);
    callee.instructions[3].op = OpV0::Load {
        slot: narrata_core::program::SlotRefV0::Local(LocalId::from_u128(10)),
        next: InstructionId::from_u128(205),
    };
    callee.instructions[4].id = InstructionId::from_u128(205);
    let target = load_program(
        &encode_program_artifact(&target_artifact),
        &Default::default(),
    )
    .unwrap();

    let execution = ExecutionId::from_u128(44);
    let mut state = Arc::new(narrata_core::new_execution(&source, execution).unwrap());
    state = transition(
        source.clone(),
        state,
        CheckedRuntimeInput::start(InputId::from_u128(1)),
    );
    let say = state.pending().unwrap().interaction_id();
    state = transition(
        source.clone(),
        state,
        CheckedRuntimeInput::advance(InputId::from_u128(2), say),
    );
    let (interaction_id, selection) = match state.pending().unwrap() {
        PendingInteractionV0::Choice {
            interaction_id,
            offered,
            ..
        } => (*interaction_id, offered[0].id),
        _ => panic!("expected choice"),
    };
    state = transition(
        source.clone(),
        state,
        CheckedRuntimeInput::select(InputId::from_u128(3), interaction_id, selection),
    );
    assert!(matches!(
        state.pending(),
        Some(PendingInteractionV0::Say { origin_instruction, .. })
            if *origin_instruction == InstructionId::from_u128(1)
    ));

    let source_flow = FlowId::from_u128(20);
    let target_flow = FlowId::from_u128(20);
    let instruction_locations = (1_u128..=5)
        .map(|id| {
            (
                InstructionLocation {
                    flow: source_flow,
                    instruction: InstructionId::from_u128(id),
                },
                InstructionLocation {
                    flow: target_flow,
                    instruction: InstructionId::from_u128(id + 200),
                },
            )
        })
        .collect();
    let migration = DeclarativeMigration::new(MigrationDescriptor {
        id: MigrationId::from_u128(99),
        from: source.artifact_id(),
        to: target.artifact_id(),
        accepted_snapshot_schemas: VersionRange {
            minimum: 0,
            maximum: 0,
        },
        relocations: RelocationTable {
            instruction_locations,
            local_locations: BTreeMap::from([(
                LocalLocation {
                    flow: source_flow,
                    local: LocalId::from_u128(1),
                },
                LocalLocation {
                    flow: target_flow,
                    local: LocalId::from_u128(10),
                },
            )]),
            ..RelocationTable::default()
        },
        recovery_points: BTreeMap::new(),
        recovery_locations: BTreeMap::new(),
    })
    .unwrap();
    let (migrated, report) = migrate_checked_state(
        &migration,
        &state,
        &source,
        &target,
        0,
        MigrationOptions::default(),
        &Default::default(),
    )
    .unwrap();
    assert!(matches!(
        migrated.pending(),
        Some(PendingInteractionV0::Say { origin_instruction, .. })
            if *origin_instruction == InstructionId::from_u128(201)
    ));
    let frame = migrated.vm().unwrap().frames.last().unwrap();
    assert!(frame.locals.contains_key(&LocalId::from_u128(10)));
    assert!(!frame.locals.contains_key(&LocalId::from_u128(1)));
    assert!(
        report
            .relocations
            .iter()
            .any(|entry| entry.kind == "instruction")
    );
}

#[test]
fn lossy_recovery_and_barrier_crossing_require_separate_confirmation() {
    let (source, target, _) = programs();
    let state = transition(
        source.clone(),
        Arc::new(narrata_core::new_execution(&source, ExecutionId::from_u128(55)).unwrap()),
        CheckedRuntimeInput::start(InputId::from_u128(1)),
    );
    let recovery = RecoveryPoint {
        checkpoint: RecoveryCheckpointId::from_u128(1),
        target_flow: FlowId::from_u128(1),
        target_instruction: InstructionId::from_u128(11),
        preserve_globals: false,
        crosses_external_effect_barrier: true,
    };
    let migration = DeclarativeMigration::new(MigrationDescriptor {
        id: MigrationId::from_u128(100),
        from: source.artifact_id(),
        to: target.artifact_id(),
        accepted_snapshot_schemas: VersionRange {
            minimum: 0,
            maximum: 0,
        },
        relocations: RelocationTable::default(),
        recovery_points: BTreeMap::from([
            (InstructionId::from_u128(1), recovery),
            (InstructionId::from_u128(2), recovery),
        ]),
        recovery_locations: BTreeMap::new(),
    })
    .unwrap();
    assert_eq!(
        migrate_checked_state(
            &migration,
            &state,
            &source,
            &target,
            0,
            MigrationOptions::default(),
            &Default::default(),
        )
        .unwrap_err(),
        MigrationError::RecoveryConfirmationRequired
    );
    assert_eq!(
        migrate_checked_state(
            &migration,
            &state,
            &source,
            &target,
            0,
            MigrationOptions {
                allow_lossy_recovery: true,
                ..MigrationOptions::default()
            },
            &Default::default(),
        )
        .unwrap_err(),
        MigrationError::BarrierConfirmationRequired
    );
    let (migrated, report) = migrate_checked_state(
        &migration,
        &state,
        &source,
        &target,
        0,
        MigrationOptions {
            allow_lossy_recovery: true,
            allow_cross_barrier_recovery: true,
            ..MigrationOptions::default()
        },
        &Default::default(),
    )
    .unwrap();
    assert!(matches!(
        migrated.status,
        narrata_core::runtime::RuntimeStatusV0::Ready { ref vm }
            if vm.frames[0].instruction == InstructionId::from_u128(11)
    ));
    assert_eq!(report.recoveries.len(), 1);
    assert!(report.is_lossy());
}

#[test]
fn pending_effect_rekey_requires_confirmation_and_reports_new_identity() {
    let source_artifact = recorded_query_v0().unwrap();
    let source = load_program(
        &encode_program_artifact(&source_artifact),
        &Default::default(),
    )
    .unwrap();
    let mut target_artifact = source_artifact;
    let flow = &mut target_artifact.flows[0];
    flow.entry = InstructionId::from_u128(11);
    flow.instructions[0].id = InstructionId::from_u128(11);
    if let OpV0::Const { next, .. } = &mut flow.instructions[0].op {
        *next = InstructionId::from_u128(12);
    }
    flow.instructions[1].id = InstructionId::from_u128(12);
    if let OpV0::Effect { next, .. } = &mut flow.instructions[1].op {
        *next = InstructionId::from_u128(13);
    }
    flow.instructions[2].id = InstructionId::from_u128(13);
    if let OpV0::Store { next, .. } = &mut flow.instructions[2].op {
        *next = InstructionId::from_u128(14);
    }
    flow.instructions[3].id = InstructionId::from_u128(14);
    if let OpV0::Say { next, .. } = &mut flow.instructions[3].op {
        *next = InstructionId::from_u128(15);
    }
    flow.instructions[4].id = InstructionId::from_u128(15);
    let target = load_program(
        &encode_program_artifact(&target_artifact),
        &Default::default(),
    )
    .unwrap();
    let parent =
        Arc::new(narrata_core::new_execution(&source, ExecutionId::from_u128(77)).unwrap());
    let state = match begin_transition_with_parent_commit(
        source.clone(),
        parent,
        CommitId::from_bytes([7; 32]),
        CheckedRuntimeInput::start(InputId::from_u128(1)),
        Default::default(),
    )
    .unwrap()
    .run_slice(SliceBudget::unlimited())
    {
        SliceOutcome::Completed(draft) => draft.into_next_state(),
        other => panic!("unexpected transition outcome: {other:?}"),
    };
    let original_effect = state.pending_effect().unwrap().request.id;
    let migration = DeclarativeMigration::new(MigrationDescriptor {
        id: MigrationId::from_u128(101),
        from: source.artifact_id(),
        to: target.artifact_id(),
        accepted_snapshot_schemas: VersionRange {
            minimum: 0,
            maximum: 0,
        },
        relocations: RelocationTable {
            instructions: (1_u128..=5)
                .map(|id| {
                    (
                        InstructionId::from_u128(id),
                        InstructionId::from_u128(id + 10),
                    )
                })
                .collect(),
            ..RelocationTable::default()
        },
        recovery_points: BTreeMap::new(),
        recovery_locations: BTreeMap::new(),
    })
    .unwrap();
    assert_eq!(
        migrate_checked_state(
            &migration,
            &state,
            &source,
            &target,
            0,
            MigrationOptions::default(),
            &Default::default(),
        )
        .unwrap_err(),
        MigrationError::EffectRekeyConfirmationRequired
    );
    let (migrated, report) = migrate_checked_state(
        &migration,
        &state,
        &source,
        &target,
        0,
        MigrationOptions {
            allow_effect_rekey: true,
            ..MigrationOptions::default()
        },
        &Default::default(),
    )
    .unwrap();
    assert_ne!(
        migrated.pending_effect().unwrap().request.id,
        original_effect
    );
    assert!(report.rekeyed_pending_effect);
}

#[test]
fn statechart_pending_effect_migrates_without_spurious_rekey_when_identity_is_stable() {
    let source_artifact = statechart_parallel_history_v0().unwrap();
    let source = load_program(
        &encode_program_artifact(&source_artifact),
        &Default::default(),
    )
    .unwrap();
    let mut target_artifact = source_artifact;
    target_artifact.constants.push(narrata_core::Value::Null);
    let target = load_program(
        &encode_program_artifact(&target_artifact),
        &Default::default(),
    )
    .unwrap();
    let parent =
        Arc::new(narrata_core::new_execution(&source, ExecutionId::from_u128(88)).unwrap());
    let state = match begin_transition_with_parent_commit(
        source.clone(),
        parent,
        CommitId::from_bytes([8; 32]),
        CheckedRuntimeInput::start(InputId::from_u128(1)),
        Default::default(),
    )
    .unwrap()
    .run_slice(SliceBudget::unlimited())
    {
        SliceOutcome::Completed(draft) => draft.into_next_state(),
        other => panic!("unexpected transition outcome: {other:?}"),
    };
    let original_effect = state.pending_effect().unwrap().request.id;
    let original_chart = state.statechart.clone();
    let migration = DeclarativeMigration::new(MigrationDescriptor {
        id: MigrationId::from_u128(102),
        from: source.artifact_id(),
        to: target.artifact_id(),
        accepted_snapshot_schemas: VersionRange {
            minimum: 0,
            maximum: 0,
        },
        relocations: RelocationTable::default(),
        recovery_points: BTreeMap::new(),
        recovery_locations: BTreeMap::new(),
    })
    .unwrap();
    let (migrated, report) = migrate_checked_state(
        &migration,
        &state,
        &source,
        &target,
        0,
        MigrationOptions::default(),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(migrated.statechart, original_chart);
    assert_eq!(
        migrated.pending_effect().unwrap().request.id,
        original_effect
    );
    assert!(!report.rekeyed_pending_effect);
}

fn transition(
    program: Arc<narrata_core::CheckedProgram>,
    state: Arc<narrata_core::RuntimeStateV0>,
    input: CheckedRuntimeInput,
) -> Arc<narrata_core::RuntimeStateV0> {
    match begin_transition(program, state, input, Default::default())
        .unwrap()
        .run_slice(SliceBudget::unlimited())
    {
        SliceOutcome::Completed(draft) => Arc::new(draft.into_next_state()),
        other => panic!("unexpected transition outcome: {other:?}"),
    }
}
