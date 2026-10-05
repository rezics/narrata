#![allow(clippy::panic, clippy::unwrap_used)]

//! Program format 1 and Snapshot schema 1 (ADR 0018): presentation names content-table entries,
//! never text, and checked decoding refuses every other shape.

use std::sync::Arc;

use narrata_core::{
    ExecutionId, InputId,
    codec::{DecodeError, ObjectKind, encode_envelope},
    diagnostic::{
        DiagnosticCode,
        codes::{CONTROL_FLOW_INVALID, KIND_MISMATCH, MISSING_REFERENCE},
    },
    load_program,
    program::{
        ConstIndex, ContentEntryV1, ContentIndex, ContentOperand, OpV0, ProgramArtifactV0,
        ProgramLoadError, Segment, encode_program_artifact, validate_program,
    },
    runtime::{
        CheckedRuntimeInput, ContentView, DraftResult, PendingContent, PendingInteractionV0,
        RuntimeStatusV0, SliceBudget, new_execution,
    },
    scene::SceneState,
    snapshot::{SnapshotExportError, SnapshotRestoreError, export_snapshot, restore_snapshot},
    version::{SNAPSHOT_SCHEMA_V0, SNAPSHOT_SCHEMA_V1},
};
use narrata_testkit::{
    backend::{ConformanceBackend, NativeBackend},
    generator::{branch_call_choice_v1, hello_v0, hello_v1, scene_reconcile_v1, story_reference},
};

fn codes(artifact: ProgramArtifactV0) -> Vec<DiagnosticCode> {
    validate_program(artifact, &Default::default())
        .unwrap_err()
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

fn decode_error(bytes: &[u8]) -> DecodeError {
    match load_program(bytes, &Default::default()).unwrap_err() {
        ProgramLoadError::Decode(error) => error,
        ProgramLoadError::Validation(errors) => panic!("expected a decode error, got {errors:?}"),
    }
}

fn payload(artifact: &ProgramArtifactV0) -> Vec<u8> {
    encode_program_artifact(artifact)[56..].to_vec()
}

#[test]
fn format_1_operands_name_content_entries_of_the_kind_their_role_needs() {
    let mut reference_body = hello_v1();
    reference_body.content[0] = ContentEntryV1::Ref(story_reference("hello").unwrap());
    assert_eq!(codes(reference_body), [KIND_MISMATCH]);

    let mut segment_label = branch_call_choice_v1();
    let OpV0::Choice { choices, .. } = &mut segment_label.flows[0].instructions[11].op else {
        panic!("expected the Choice");
    };
    choices[0].label = ContentOperand::Content(ContentIndex(0));
    assert_eq!(codes(segment_label), [KIND_MISMATCH]);

    let mut missing = hello_v1();
    missing.content.clear();
    assert_eq!(codes(missing), [MISSING_REFERENCE]);

    let mut constant_text = hello_v1();
    constant_text.constants.push("Hello".into());
    let OpV0::Say { text, .. } = &mut constant_text.flows[0].instructions[0].op else {
        panic!("expected the Say");
    };
    *text = ContentOperand::Constant(ConstIndex(0));
    assert_eq!(codes(constant_text), [KIND_MISMATCH]);

    let mut content_in_format_0 = hello_v0();
    let OpV0::Say { text, .. } = &mut content_in_format_0.flows[0].instructions[0].op else {
        panic!("expected the Say");
    };
    *text = ContentOperand::Content(ContentIndex(0));
    assert_eq!(codes(content_in_format_0), [KIND_MISMATCH]);

    let mut table_in_format_0 = hello_v0();
    table_in_format_0.content = hello_v1().content;
    assert_eq!(codes(table_in_format_0.clone()), [CONTROL_FLOW_INVALID]);
    assert!(matches!(
        decode_error(&encode_program_artifact(&table_in_format_0)),
        DecodeError::Schema(_)
    ));
}

#[test]
fn program_format_1_decoding_is_checked() {
    let hello = hello_v1();
    let program = load_program(&encode_program_artifact(&hello), &Default::default()).unwrap();
    assert_eq!(program.artifact(), &hello);
    assert_ne!(
        program.artifact_id(),
        load_program(&encode_program_artifact(&hello_v0()), &Default::default())
            .unwrap()
            .artifact_id()
    );

    // The envelope schema and the payload's format version must agree.
    let relabelled = encode_envelope(ObjectKind::Program, 0, &payload(&hello));
    assert!(matches!(decode_error(&relabelled), DecodeError::Schema(_)));
    let unknown = encode_envelope(ObjectKind::Program, 2, &payload(&hello));
    assert!(matches!(
        decode_error(&unknown),
        DecodeError::UnsupportedVersion { .. }
    ));

    // The content entry `[1, Segment]` sits right after map key 8 and its one-entry array.
    let bytes = payload(&hello);
    let entry = bytes
        .windows(4)
        .position(|window| window == [0x08, 0x81, 0x82, 0x01])
        .unwrap();
    let mut unknown_tag = bytes.clone();
    unknown_tag[entry + 3] = 0x02;
    let unknown_tag = encode_envelope(ObjectKind::Program, 1, &unknown_tag);
    assert!(matches!(decode_error(&unknown_tag), DecodeError::Schema(_)));

    let mut long_entry = bytes;
    long_entry[entry + 2] = 0x83;
    let long_entry = encode_envelope(ObjectKind::Program, 1, &long_entry);
    assert!(matches!(decode_error(&long_entry), DecodeError::Schema(_)));
}

#[test]
fn format_1_states_record_content_indices_and_views_name_references() {
    let program = load_program(
        &encode_program_artifact(&branch_call_choice_v1()),
        &Default::default(),
    )
    .unwrap();
    let state = Arc::new(new_execution(&program, ExecutionId::from_u128(7)).unwrap());
    assert_eq!(state.snapshot_schema, SNAPSHOT_SCHEMA_V1);
    assert_eq!(state.scene, None);
    let say = transition(
        &program,
        state,
        CheckedRuntimeInput::start(InputId::from_u128(1)),
    );
    let DraftResult::AwaitSay(view) = say.result() else {
        panic!("expected a Say");
    };
    assert_eq!(
        view.text,
        ContentView::Segment(Segment::unit(story_reference("choose-a-path").unwrap()))
    );
    assert_eq!(view.occurrence, 0);
    assert!(matches!(
        say.next_state().pending(),
        Some(PendingInteractionV0::Say {
            text: PendingContent::Content(ContentIndex(0)),
            speaker: None,
            ..
        })
    ));
    let interaction = say.next_state().pending().unwrap().interaction_id();
    let choice = transition(
        &program,
        Arc::new(say.next_state().clone()),
        CheckedRuntimeInput::advance(InputId::from_u128(2), interaction),
    );
    let DraftResult::AwaitChoice(view) = choice.result() else {
        panic!("expected a Choice");
    };
    assert_eq!(view.occurrence, 1);
    assert_eq!(
        view.prompt,
        Some(ContentView::Ref(story_reference("choose-a-path").unwrap()))
    );
    assert_eq!(
        view.choices[0].label,
        ContentView::Ref(story_reference("continue").unwrap())
    );
    let snapshot = export_snapshot(choice.next_state()).unwrap();
    assert_eq!(&snapshot[..8], b"NARRATA\0");
    assert_eq!(
        &restore_snapshot(&snapshot, &program, &Default::default()).unwrap(),
        choice.next_state()
    );
}

#[test]
fn schema_1_restore_and_export_refuse_other_shapes() {
    let program = load_program(&encode_program_artifact(&hello_v1()), &Default::default()).unwrap();
    let initial = Arc::new(new_execution(&program, ExecutionId::from_u128(8)).unwrap());
    let say = transition(
        &program,
        initial,
        CheckedRuntimeInput::start(InputId::from_u128(1)),
    );
    let state = say.next_state();

    let mut text = state.clone();
    if let RuntimeStatusV0::Awaiting {
        pending: PendingInteractionV0::Say { text, .. },
        ..
    } = &mut text.status
    {
        *text = PendingContent::LegacyText("Hello".into());
    }
    assert_eq!(
        export_snapshot(&text).unwrap_err(),
        SnapshotExportError::SchemaMismatch
    );

    let mut legacy = state.clone();
    legacy.snapshot_schema = SNAPSHOT_SCHEMA_V0;
    assert_eq!(
        export_snapshot(&legacy).unwrap_err(),
        SnapshotExportError::SchemaMismatch
    );

    let mut other_entry = state.clone();
    if let RuntimeStatusV0::Awaiting {
        pending: PendingInteractionV0::Say { text, .. },
        ..
    } = &mut other_entry.status
    {
        *text = PendingContent::Content(ContentIndex(1));
    }
    assert!(matches!(
        restore_snapshot(
            &export_snapshot(&other_entry).unwrap(),
            &program,
            &Default::default()
        ),
        Err(SnapshotRestoreError::InvalidState(_))
    ));

    let mut scene = state.clone();
    scene.scene = Some(SceneState::default());
    assert_eq!(
        restore_snapshot(
            &export_snapshot(&scene).unwrap(),
            &program,
            &Default::default()
        ),
        Err(SnapshotRestoreError::InvalidState("scene presence"))
    );

    let legacy_program =
        load_program(&encode_program_artifact(&hello_v0()), &Default::default()).unwrap();
    let mut wrong_program = state.clone();
    wrong_program.program_artifact_id = legacy_program.artifact_id();
    assert!(matches!(
        restore_snapshot(
            &export_snapshot(&wrong_program).unwrap(),
            &legacy_program,
            &Default::default()
        ),
        Err(SnapshotRestoreError::Incompatible(_))
    ));
}

#[test]
fn programs_that_reconcile_scenes_keep_the_scene_in_schema_1() {
    let program = load_program(
        &encode_program_artifact(&scene_reconcile_v1().unwrap()),
        &Default::default(),
    )
    .unwrap();
    assert!(program.uses_scene());
    let state = Arc::new(new_execution(&program, ExecutionId::from_u128(9)).unwrap());
    assert_eq!(state.scene, Some(SceneState::default()));
    let say = transition(
        &program,
        state,
        CheckedRuntimeInput::start(InputId::from_u128(1)),
    );
    assert!(
        say.next_state()
            .scene
            .as_ref()
            .is_some_and(|scene| !scene.layers().is_empty())
    );
    let mut dropped = say.next_state().clone();
    dropped.scene = None;
    assert_eq!(
        restore_snapshot(
            &export_snapshot(&dropped).unwrap(),
            &program,
            &Default::default()
        ),
        Err(SnapshotRestoreError::InvalidState("scene presence"))
    );
}

fn transition(
    program: &Arc<narrata_core::CheckedProgram>,
    state: Arc<narrata_core::RuntimeStateV0>,
    input: CheckedRuntimeInput,
) -> narrata_core::TransitionDraft {
    NativeBackend
        .transition(
            program.clone(),
            state,
            input,
            Default::default(),
            SliceBudget::unlimited(),
        )
        .unwrap()
}
