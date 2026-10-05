#![allow(clippy::panic, clippy::unwrap_used)]

//! The ADR 0018 upgrade from Program format 0 to format 1 and the migration that moves saves.

use std::{collections::BTreeMap, sync::Arc};

use narrata_core::{
    ChoiceId, ExecutionId, InputId, MigrationId, ProgramId, Value,
    migration::{
        DeclarativeMigration, MigrationDescriptor, MigrationError, MigrationOptions,
        RelocationTable, TrustedMigration, VersionRange, migrate_checked_state,
    },
    program::{
        CheckedProgram, ConstIndex, ContentEntryV1, ContentOperand, OpV0, ProgramArtifactV0,
        encode_program_artifact, load_program,
    },
    runtime::{
        CheckedRuntimeInput, DraftResult, PendingContent, PendingInteractionV0, RuntimeStateV0,
        SliceBudget, new_execution,
    },
    snapshot::{export_snapshot, restore_snapshot},
    upgrade::{
        LocalContentPack, UNDETERMINED_LANGUAGE, UpgradeError, format_upgrade_migration,
        upgrade_program_v0, upgraded_text_reference,
    },
    version::{PROGRAM_FORMAT_V1, SNAPSHOT_SCHEMA_V1},
};
use narrata_testkit::{
    backend::{ConformanceBackend, NativeBackend},
    generator::{
        barrier_command_v0, barrier_command_v1, branch_call_choice_v0, branch_call_choice_v1,
        hello_v0, hello_v1, recorded_query_v0, recorded_query_v1, scene_reconcile_v0,
        scene_reconcile_v1, statechart_parallel_history_v0, statechart_parallel_history_v1,
        story_content,
    },
};

fn checked(artifact: &ProgramArtifactV0) -> Arc<CheckedProgram> {
    load_program(&encode_program_artifact(artifact), &Default::default()).unwrap()
}

/// The kind and text of every content entry.
fn resolved(artifact: &ProgramArtifactV0, pack: &LocalContentPack) -> Vec<(&'static str, String)> {
    artifact
        .content
        .iter()
        .map(|entry| {
            let reference = match entry {
                ContentEntryV1::Ref(reference) => reference,
                ContentEntryV1::Segment(segment) => &segment.unit,
            };
            assert_eq!(reference.provider.as_str(), pack.provider);
            let text = pack.entries[reference.key.as_str()].text.clone();
            (entry.kind_name(), text)
        })
        .collect()
}

#[test]
fn upgrade_moves_text_to_a_local_pack_and_matches_the_authored_stories() {
    let pairs = [
        (hello_v0(), hello_v1()),
        (branch_call_choice_v0(), branch_call_choice_v1()),
        (scene_reconcile_v0().unwrap(), scene_reconcile_v1().unwrap()),
        (recorded_query_v0().unwrap(), recorded_query_v1().unwrap()),
        (barrier_command_v0().unwrap(), barrier_command_v1().unwrap()),
        (
            statechart_parallel_history_v0().unwrap(),
            statechart_parallel_history_v1().unwrap(),
        ),
    ];
    for (legacy, authored) in pairs {
        let upgraded = upgrade_program_v0(&checked(&legacy), "en").unwrap();
        assert_eq!(upgraded.artifact.format_version, PROGRAM_FORMAT_V1);
        assert_eq!(
            resolved(&upgraded.artifact, &upgraded.content),
            resolved(&authored, &story_content())
        );
        // Apart from the keys the upgrade derives from text, it is the authored format 1 story.
        let mut renamed = upgraded.artifact.clone();
        renamed.content = authored.content.clone();
        assert_eq!(renamed, authored);
        checked(&upgraded.artifact);
    }
}

#[test]
fn upgrade_is_deterministic_and_references_depend_only_on_text() {
    let program = checked(&branch_call_choice_v0());
    let first = upgrade_program_v0(&program, UNDETERMINED_LANGUAGE).unwrap();
    let second = upgrade_program_v0(&program, "fr-CA").unwrap();
    assert_eq!(first.artifact, second.artifact);
    assert_eq!(first.content.entries, second.content.entries);
    assert_eq!(second.content.language, "fr-CA");
    assert_eq!(first.content.provider, "local");
    let hello = upgraded_text_reference("Hello").unwrap();
    assert_eq!(hello, upgraded_text_reference("Hello").unwrap());
    assert_ne!(hello, upgraded_text_reference("Hello!").unwrap());
    assert!(hello.key.as_str().starts_with("stage5-"));
    assert_eq!(hello.key.as_str().len(), "stage5-".len() + 32);
    let upgraded_hello = upgrade_program_v0(&checked(&hello_v0()), "en").unwrap();
    assert!(
        upgraded_hello
            .content
            .entries
            .contains_key(hello.key.as_str())
    );
}

#[test]
fn upgrade_escapes_braces_keeps_rule_constants_and_drops_presentation_text() {
    let mut legacy = branch_call_choice_v0();
    legacy.constants[2] = Value::from("Pick {one} of {{two}}");
    let upgraded = upgrade_program_v0(&checked(&legacy), "en").unwrap();
    let key = upgraded_text_reference("Pick {one} of {{two}}").unwrap();
    assert_eq!(
        upgraded.content.entries[key.key.as_str()].text,
        "Pick {{one}} of {{{{two}}}}"
    );
    assert_eq!(
        upgraded.artifact.constants,
        [
            Value::I64(2),
            Value::Bool(false),
            Value::from("argument"),
            Value::I64(41),
            Value::I64(1),
        ]
    );
    let pushes = upgraded
        .artifact
        .flows
        .iter()
        .flat_map(|flow| &flow.instructions)
        .filter_map(|instruction| match instruction.op {
            OpV0::Const { constant, .. } => Some(constant),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        pushes,
        [0, 1, 0, 2, 4, 3].map(ConstIndex),
        "Const instructions name the renumbered rule constants"
    );
}

#[test]
fn upgrade_rejects_format_1_and_bad_languages() {
    assert_eq!(
        upgrade_program_v0(&checked(&hello_v1()), "en").unwrap_err(),
        UpgradeError::NotFormatV0
    );
    assert_eq!(
        upgrade_program_v0(&checked(&hello_v0()), "not a tag").unwrap_err(),
        UpgradeError::Language
    );
    assert_eq!(
        upgrade_program_v0(&checked(&hello_v0()), "").unwrap_err(),
        UpgradeError::Language
    );
}

#[test]
fn format_upgrade_migration_moves_pending_interactions_and_keeps_their_ids() {
    let source = checked(&branch_call_choice_v0());
    let target = checked(&upgrade_program_v0(&source, "en").unwrap().artifact);
    let migration = format_upgrade_migration(&source, &target).unwrap();
    assert_eq!(
        migration.descriptor().id,
        format_upgrade_migration(&source, &target)
            .unwrap()
            .descriptor()
            .id
    );

    let initial = Arc::new(new_execution(&source, ExecutionId::from_u128(11)).unwrap());
    let say = run(
        &source,
        initial,
        CheckedRuntimeInput::start(InputId::from_u128(1)),
    );
    let say_id = say.pending().unwrap().interaction_id();
    let choice = run(
        &source,
        Arc::new(say.clone()),
        CheckedRuntimeInput::advance(InputId::from_u128(2), say_id),
    );
    for state in [&say, &choice] {
        let upgraded = upgrade_state(&migration, state, &source, &target);
        assert_eq!(upgraded.snapshot_schema, SNAPSHOT_SCHEMA_V1);
        assert_eq!(upgraded.scene, None);
        assert_eq!(upgraded.program_artifact_id, target.artifact_id());
        assert_eq!(
            upgraded.pending().unwrap().interaction_id(),
            state.pending().unwrap().interaction_id()
        );
        assert!(
            upgraded
                .pending()
                .unwrap()
                .contents()
                .all(|content| matches!(content, PendingContent::Content(_)))
        );
        let restored = restore_snapshot(
            &export_snapshot(&upgraded).unwrap(),
            &target,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(restored, upgraded);
    }

    // The upgraded save continues to the end on format 1.
    let upgraded = upgrade_state(&migration, &choice, &source, &target);
    let choice_id = match upgraded.pending().unwrap() {
        PendingInteractionV0::Choice { interaction_id, .. } => *interaction_id,
        PendingInteractionV0::Say { .. } => panic!("expected the Choice"),
    };
    let callee = run(
        &target,
        Arc::new(upgraded),
        CheckedRuntimeInput::select(InputId::from_u128(3), choice_id, ChoiceId::from_u128(1)),
    );
    let callee_id = callee.pending().unwrap().interaction_id();
    let finished = NativeBackend
        .transition(
            target.clone(),
            Arc::new(callee),
            CheckedRuntimeInput::advance(InputId::from_u128(4), callee_id),
            Default::default(),
            SliceBudget::unlimited(),
        )
        .unwrap();
    assert!(matches!(finished.result(), DraftResult::Finished(_)));
}

#[test]
fn format_upgrade_keeps_used_scenes_and_drops_default_ones() {
    let source = checked(&scene_reconcile_v0().unwrap());
    let target = checked(&upgrade_program_v0(&source, "en").unwrap().artifact);
    let migration = format_upgrade_migration(&source, &target).unwrap();
    let initial = Arc::new(new_execution(&source, ExecutionId::from_u128(12)).unwrap());
    let say = run(
        &source,
        initial,
        CheckedRuntimeInput::start(InputId::from_u128(1)),
    );
    let upgraded = upgrade_state(&migration, &say, &source, &target);
    assert_eq!(upgraded.scene, say.scene);

    let hello = checked(&hello_v0());
    let hello_target = checked(&upgrade_program_v0(&hello, "en").unwrap().artifact);
    let hello_migration = format_upgrade_migration(&hello, &hello_target).unwrap();
    let mut decorated = new_execution(&hello, ExecutionId::from_u128(13)).unwrap();
    decorated.scene = say.scene.clone();
    assert_eq!(
        migrate_checked_state(
            &hello_migration,
            &decorated,
            &hello,
            &hello_target,
            MigrationOptions::default(),
            &Default::default(),
        )
        .unwrap_err(),
        MigrationError::SceneWithoutTarget
    );
}

#[test]
fn format_upgrade_refuses_other_targets_and_never_moves_backwards() {
    let source = checked(&hello_v0());
    let authored = checked(&hello_v1());
    assert_eq!(
        format_upgrade_migration(&source, &authored).unwrap_err(),
        MigrationError::ArtifactMismatch
    );
    assert_eq!(
        format_upgrade_migration(&source, &source).unwrap_err(),
        MigrationError::ArtifactMismatch
    );

    let backwards = DeclarativeMigration::new(MigrationDescriptor {
        id: MigrationId::from_u128(5),
        from: authored.artifact_id(),
        to: source.artifact_id(),
        accepted_snapshot_schemas: VersionRange {
            minimum: 0,
            maximum: 1,
        },
        relocations: RelocationTable::default(),
        recovery_points: BTreeMap::new(),
        recovery_locations: BTreeMap::new(),
    })
    .unwrap();
    assert_eq!(authored.artifact().program_id, ProgramId::from_u128(1));
    let state = new_execution(&authored, ExecutionId::from_u128(14)).unwrap();
    assert_eq!(
        migrate_checked_state(
            &backwards,
            &state,
            &authored,
            &source,
            MigrationOptions::default(),
            &Default::default(),
        )
        .unwrap_err(),
        MigrationError::FormatDowngrade
    );
}

fn upgrade_state(
    migration: &DeclarativeMigration,
    state: &RuntimeStateV0,
    source: &CheckedProgram,
    target: &CheckedProgram,
) -> RuntimeStateV0 {
    let (upgraded, report) = migrate_checked_state(
        migration,
        state,
        source,
        target,
        MigrationOptions::default(),
        &Default::default(),
    )
    .unwrap();
    assert!(!report.is_lossy());
    assert!(!report.rekeyed_pending_effect);
    upgraded
}

fn run(
    program: &Arc<CheckedProgram>,
    state: Arc<RuntimeStateV0>,
    input: CheckedRuntimeInput,
) -> RuntimeStateV0 {
    NativeBackend
        .transition(
            program.clone(),
            state,
            input,
            Default::default(),
            SliceBudget::unlimited(),
        )
        .unwrap()
        .into_next_state()
}

#[test]
fn format_1_operands_never_name_constants_after_upgrade() {
    let upgraded = upgrade_program_v0(&checked(&branch_call_choice_v0()), "en").unwrap();
    for instruction in upgraded
        .artifact
        .flows
        .iter()
        .flat_map(|flow| &flow.instructions)
    {
        let operands: Vec<ContentOperand> = match &instruction.op {
            OpV0::Say { speaker, text, .. } => speaker.iter().copied().chain([*text]).collect(),
            OpV0::Choice { prompt, choices } => prompt
                .iter()
                .copied()
                .chain(choices.iter().map(|choice| choice.label))
                .collect(),
            _ => Vec::new(),
        };
        assert!(
            operands
                .iter()
                .all(|operand| matches!(operand, ContentOperand::Content(_)))
        );
    }
}
