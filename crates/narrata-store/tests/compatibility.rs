#![allow(clippy::panic, clippy::unwrap_used)]

#[macro_use]
mod support;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
};

use narrata_core::{
    CheckedProgram, ChoiceId, CommitId, ContentRef, InputId, ObjectId, ReceiptId,
    codec::{ObjectKind, sha256},
    migration::MigrationOptions,
    program::{encode_program_artifact, load_program},
    runtime::{
        CheckedRuntimeInput, ContentView, DraftResult, PendingContent, PendingInteractionV0,
        RuntimeStateV0, SliceBudget, SliceOutcome, begin_transition,
        begin_transition_with_parent_commit, pending_view,
    },
    snapshot::{restore_snapshot, state_digest},
    upgrade::{LocalContentPack, format_upgrade_migration, upgrade_program_v0},
    version::SNAPSHOT_SCHEMA_V1,
};
use narrata_store::{
    BundleLimits, CheckedObject, CheckpointBundle, CommitV1, MigrationRegistry, ProgramRegistry,
    RefKey, RefName, SaveStore, TimelineArchiveBundle, TransitionReceiptV1, apply_migration,
    dry_run_migration, load_commit,
};
use serde::Deserialize;
use support::Backend;

#[derive(Deserialize)]
struct Manifest {
    artifacts: BTreeMap<String, Artifact>,
    commit_id: String,
    receipt_id: String,
    expected_continuation: Expected,
}

#[derive(Deserialize)]
struct Artifact {
    sha256: String,
}

#[derive(Deserialize)]
struct Expected {
    kind: String,
    text: String,
}

fn frozen_stage5_corpus_loads_imports_and_continues<B: Backend>() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/compat/stage5-v0");
    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let mut decoded = BTreeMap::new();
    for (name, expected) in &manifest.artifacts {
        let text = std::fs::read_to_string(root.join(name)).unwrap();
        let bytes = hex::decode(text.trim()).unwrap();
        assert_eq!(hex::encode(sha256(&bytes)), expected.sha256, "{name}");
        decoded.insert(name.as_str(), bytes);
    }

    let program = load_program(&decoded["program-v0.hex"], &Default::default()).unwrap();
    let state =
        restore_snapshot(&decoded["snapshot-v0.hex"], &program, &Default::default()).unwrap();
    assert_eq!(manifest.expected_continuation.kind, "say");
    assert!(matches!(
        state.pending(),
        Some(PendingInteractionV0::Say { text, .. })
            if *text == PendingContent::LegacyText(manifest.expected_continuation.text.as_str().into())
    ));
    let receipt_object = CheckedObject::from_bytes(
        &decoded["receipt-v1.hex"],
        ObjectKind::Receipt,
        1,
        1024 * 1024,
    )
    .unwrap();
    let receipt = TransitionReceiptV1::decode(receipt_object.payload()).unwrap();
    assert_eq!(
        ReceiptId::from_str(&manifest.receipt_id).unwrap(),
        ReceiptId::from_bytes(*receipt_object.id().as_bytes())
    );
    let commit_object = CheckedObject::from_bytes(
        &decoded["commit-v1.hex"],
        ObjectKind::Commit,
        1,
        1024 * 1024,
    )
    .unwrap();
    let commit = CommitV1::decode(commit_object.payload()).unwrap();
    assert_eq!(
        commit.cause,
        narrata_store::CommitCauseV1::RuntimeTransition(receipt_object.receipt_id().unwrap())
    );
    assert_eq!(commit.snapshot, receipt.next_snapshot);

    let checkpoint = CheckpointBundle::from_bytes(
        &decoded["checkpoint-bundle-v1.hex"],
        BundleLimits::default(),
    )
    .unwrap();
    assert_eq!(
        checkpoint.manifest.root,
        CommitId::from_str(&manifest.commit_id).unwrap()
    );
    let mut store = B::store();
    let active = RefKey::active(RefName::new("compat-loaded").unwrap()).unwrap();
    checkpoint.import(&mut store, active, None, 0).unwrap();
    let loaded = load_commit(&store, commit_object.commit_id().unwrap(), &program).unwrap();
    let interaction = loaded.state.pending().unwrap().interaction_id();
    let mut outcome = begin_transition_with_parent_commit(
        Arc::clone(&program),
        Arc::new(loaded.state),
        loaded.id,
        CheckedRuntimeInput::advance(InputId::from_u128(501), interaction),
        Default::default(),
    )
    .unwrap()
    .run_slice(SliceBudget::unlimited());
    loop {
        match outcome {
            SliceOutcome::Completed(draft) => {
                assert!(matches!(draft.result(), DraftResult::Finished(_)));
                break;
            }
            SliceOutcome::Yielded { runner, .. } => {
                outcome = runner.run_slice(SliceBudget::unlimited());
            }
            SliceOutcome::Faulted(error) => panic!("continuation faulted: {error}"),
        }
    }

    let timeline = TimelineArchiveBundle::from_bytes(
        &decoded["timeline-archive-v1.hex"],
        BundleLimits::default(),
    )
    .unwrap();
    assert_eq!(timeline.manifest.execution, commit.execution);
    assert!(
        timeline
            .manifest
            .objects
            .iter()
            .map(|descriptor| descriptor.id)
            .collect::<BTreeSet<ObjectId>>()
            .contains(&ObjectId::from_bytes(*commit_object.id().as_bytes()))
    );
}

backend_tests!(
    frozen_stage5_corpus_loads_imports_and_continues,
    frozen_stage5_corpus_upgrades_to_format_1_and_runs_to_the_end,
    frozen_stage6_corpus_loads_imports_and_continues,
);

fn decoded_corpus(name: &str) -> (PathBuf, BTreeMap<String, Vec<u8>>, serde_json::Value) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/compat")
        .join(name);
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let mut decoded = BTreeMap::new();
    for (file, expected) in manifest["artifacts"].as_object().unwrap() {
        let text = std::fs::read_to_string(root.join(file)).unwrap();
        let bytes = if file.ends_with(".hex") {
            hex::decode(text.trim()).unwrap()
        } else {
            text.into_bytes()
        };
        assert_eq!(
            hex::encode(sha256(&bytes)),
            expected["sha256"].as_str().unwrap(),
            "{file}"
        );
        decoded.insert(file.clone(), bytes);
    }
    (root, decoded, manifest)
}

/// Answers every interaction (choosing `choose`) and returns how many inputs the story took.
/// The stories have no Effects, so no step needs a persistent parent Commit.
fn run_to_end(program: &Arc<CheckedProgram>, mut state: RuntimeStateV0, choose: ChoiceId) -> usize {
    for step in 0_u128..16 {
        let input = InputId::from_u128(900 + step);
        let input = match state.pending().unwrap() {
            PendingInteractionV0::Say { interaction_id, .. } => {
                CheckedRuntimeInput::advance(input, *interaction_id)
            }
            PendingInteractionV0::Choice { interaction_id, .. } => {
                CheckedRuntimeInput::select(input, *interaction_id, choose)
            }
        };
        let mut outcome = begin_transition(
            Arc::clone(program),
            Arc::new(state),
            input,
            Default::default(),
        )
        .unwrap()
        .run_slice(SliceBudget::unlimited());
        let draft = loop {
            match outcome {
                SliceOutcome::Completed(draft) => break draft,
                SliceOutcome::Yielded { runner, .. } => {
                    outcome = runner.run_slice(SliceBudget::unlimited());
                }
                SliceOutcome::Faulted(error) => panic!("continuation faulted: {error}"),
            }
        };
        if matches!(draft.result(), DraftResult::Finished(_)) {
            return usize::try_from(step).unwrap() + 1;
        }
        state = draft.into_next_state();
    }
    panic!("the story did not finish");
}

/// ADR 0018: the frozen format 0 corpus upgrades through the ADR 0009 migration, keeps its
/// pending interaction, and continues on format 1 to the end, also after a bundle round trip.
fn frozen_stage5_corpus_upgrades_to_format_1_and_runs_to_the_end<B: Backend>() {
    let (_, decoded, manifest) = decoded_corpus("stage5-v0");
    let source = load_program(&decoded["program-v0.hex"], &Default::default()).unwrap();
    let upgraded = upgrade_program_v0(&source, "en").unwrap();
    let target = load_program(
        &encode_program_artifact(&upgraded.artifact),
        &Default::default(),
    )
    .unwrap();
    let mut programs = ProgramRegistry::new();
    programs.register(Arc::clone(&source));
    programs.register(Arc::clone(&target));
    let mut registry = MigrationRegistry::new();
    registry
        .register(Arc::new(
            format_upgrade_migration(&source, &target).unwrap(),
        ))
        .unwrap();

    let mut store = B::store();
    let commit = CommitId::from_str(manifest["commit_id"].as_str().unwrap()).unwrap();
    CheckpointBundle::from_bytes(
        &decoded["checkpoint-bundle-v1.hex"],
        BundleLimits::default(),
    )
    .unwrap()
    .import(
        &mut store,
        RefKey::active(RefName::new("stage5").unwrap()).unwrap(),
        None,
        0,
    )
    .unwrap();
    let before = load_commit(&store, commit, &source).unwrap();

    let dry_run = dry_run_migration(
        &store,
        &registry,
        &programs,
        commit,
        target.artifact_id(),
        None,
        MigrationOptions::default(),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(dry_run.target_artifact, target.artifact_id());
    assert_eq!(dry_run.reports.len(), 1);
    assert!(!dry_run.reports[0].is_lossy());
    assert!(
        store
            .get_object(ObjectId::from_bytes(
                *CheckedObject::from_bytes(
                    &encode_program_artifact(target.artifact()),
                    ObjectKind::Program,
                    1,
                    1024 * 1024
                )
                .unwrap()
                .id()
                .as_bytes()
            ))
            .unwrap()
            .is_none(),
        "a dry run writes nothing"
    );

    let applied = apply_migration(
        &mut store,
        &registry,
        &programs,
        commit,
        target.artifact_id(),
        None,
        MigrationOptions::default(),
        RefKey::save(
            RefName::new("player").unwrap(),
            RefName::new("upgraded").unwrap(),
        ),
        None,
        0,
        &Default::default(),
    )
    .unwrap();
    let loaded = load_commit(&store, applied.commit, &target).unwrap();
    assert_eq!(loaded.commit.parent, Some(commit));
    assert_eq!(loaded.state.snapshot_schema, SNAPSHOT_SCHEMA_V1);
    assert_eq!(dry_run.final_state, loaded.state);
    assert_eq!(
        loaded.state.pending().unwrap().interaction_id(),
        before.state.pending().unwrap().interaction_id()
    );
    let DraftResult::AwaitSay(view) =
        pending_view(&target, loaded.state.pending().unwrap()).unwrap()
    else {
        panic!("the corpus waits on a Say");
    };
    let ContentView::Segment(body) = &view.text else {
        panic!("format 1 shows a segment");
    };
    assert_eq!(
        upgraded.content.entries[body.unit.key.as_str()].text,
        manifest["expected_continuation"]["text"].as_str().unwrap()
    );
    assert_eq!(
        run_to_end(&target, loaded.state.clone(), ChoiceId::from_u128(1)),
        1
    );

    let exported = CheckpointBundle::export(&store, applied.commit, &BTreeSet::new())
        .unwrap()
        .to_bytes()
        .unwrap();
    let mut fresh = B::store();
    CheckpointBundle::from_bytes(&exported, BundleLimits::default())
        .unwrap()
        .import(
            &mut fresh,
            RefKey::active(RefName::new("upgraded").unwrap()).unwrap(),
            None,
            0,
        )
        .unwrap();
    let reloaded = load_commit(&fresh, applied.commit, &target).unwrap();
    assert_eq!(reloaded.state, loaded.state);
    assert_eq!(
        run_to_end(&target, reloaded.state, ChoiceId::from_u128(1)),
        1
    );
}

/// The frozen format 1 corpus (ADR 0018) reads byte for byte, imports, and continues to the end.
fn frozen_stage6_corpus_loads_imports_and_continues<B: Backend>() {
    let (root, decoded, manifest) = decoded_corpus("stage6-v0");
    let program = load_program(&decoded["program-v1.hex"], &Default::default()).unwrap();
    assert_eq!(
        program.artifact_id().to_string(),
        manifest["program_artifact_id"].as_str().unwrap()
    );
    let state =
        restore_snapshot(&decoded["snapshot-v1.hex"], &program, &Default::default()).unwrap();
    assert_eq!(state.snapshot_schema, SNAPSHOT_SCHEMA_V1);
    assert_eq!(
        state_digest(&state).to_string(),
        manifest["state_digest"].as_str().unwrap()
    );
    let expected = &manifest["expected_continuation"];
    let reference = |value: &serde_json::Value| {
        ContentView::Ref(
            ContentRef::new(
                value["provider"].as_str().unwrap(),
                value["key"].as_str().unwrap(),
            )
            .unwrap(),
        )
    };
    let DraftResult::AwaitChoice(view) = pending_view(&program, state.pending().unwrap()).unwrap()
    else {
        panic!("the corpus waits on a Choice");
    };
    assert_eq!(expected["kind"], "choice");
    assert_eq!(view.occurrence, expected["occurrence"].as_u64().unwrap());
    assert_eq!(view.prompt, Some(reference(&expected["prompt"])));
    let choices = expected["choices"].as_array().unwrap();
    assert_eq!(view.choices.len(), choices.len());
    for (item, expected) in view.choices.iter().zip(choices) {
        assert_eq!(item.id.to_string(), expected["id"].as_str().unwrap());
        assert_eq!(item.label, reference(&expected["label"]));
    }
    let content: LocalContentPack =
        serde_json::from_slice(&decoded[manifest["content_pack"].as_str().unwrap()]).unwrap();
    assert!(view.choices.iter().all(|item| match &item.label {
        ContentView::Ref(reference) => content.entries.contains_key(reference.key.as_str()),
        _ => false,
    }));

    let receipt_object = CheckedObject::from_bytes(
        &decoded["receipt-v1.hex"],
        ObjectKind::Receipt,
        1,
        1024 * 1024,
    )
    .unwrap();
    let receipt = TransitionReceiptV1::decode(receipt_object.payload()).unwrap();
    assert_eq!(
        receipt_object.receipt_id().unwrap().to_string(),
        manifest["receipt_id"].as_str().unwrap()
    );
    assert_eq!(receipt.next_state, state_digest(&state));
    let commit_object = CheckedObject::from_bytes(
        &decoded["commit-v1.hex"],
        ObjectKind::Commit,
        1,
        1024 * 1024,
    )
    .unwrap();
    let commit = CommitV1::decode(commit_object.payload()).unwrap();
    assert_eq!(commit.snapshot, receipt.next_snapshot);
    assert_eq!(commit.program, program.artifact_id());

    let checkpoint = CheckpointBundle::from_bytes(
        &decoded["checkpoint-bundle-v1.hex"],
        BundleLimits::default(),
    )
    .unwrap();
    let commit_id = CommitId::from_str(manifest["commit_id"].as_str().unwrap()).unwrap();
    assert_eq!(checkpoint.manifest.root, commit_id);
    let mut store = B::store();
    checkpoint
        .import(
            &mut store,
            RefKey::active(RefName::new("stage6").unwrap()).unwrap(),
            None,
            0,
        )
        .unwrap();
    let loaded = load_commit(&store, commit_id, &program).unwrap();
    assert_eq!(loaded.state, state);
    let select = ChoiceId::from_str(expected["select"].as_str().unwrap()).unwrap();
    assert_eq!(run_to_end(&program, loaded.state, select), 2);

    let timeline = TimelineArchiveBundle::from_bytes(
        &decoded["timeline-archive-v1.hex"],
        BundleLimits::default(),
    )
    .unwrap();
    assert_eq!(timeline.manifest.execution, commit.execution);
    assert!(
        timeline
            .manifest
            .objects
            .iter()
            .any(|descriptor| descriptor.id == ObjectId::from_bytes(*commit_id.as_bytes()))
    );
    assert!(root.join("content-pack.json").exists());
}
