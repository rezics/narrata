//! Writes the `fixtures/compat/stage6-v0` corpus (ADR 0018) into the directory given as the
//! only argument. The corpus is frozen: rerun this only to compare, never to overwrite it.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

use narrata_core::{
    ChoiceId, ExecutionId, InputId, ObjectId,
    codec::sha256,
    program::encode_program_artifact,
    runtime::{CheckedRuntimeInput, ContentView, DraftResult},
};
use narrata_store::{
    BranchId, CheckpointBundle, CommitV1, InitialRecordingMode, MemoryStore, SaveStore,
    SessionCoordinator, TimelineArchiveBundle,
};
use narrata_testkit::generator::{branch_call_choice_v1, story_content};
use serde_json::{Value, json};

fn reference(view: &ContentView) -> Value {
    match view {
        ContentView::Ref(reference) => {
            json!({ "provider": reference.provider.as_str(), "key": reference.key.as_str() })
        }
        ContentView::Segment(_) | ContentView::LegacyText(_) => {
            unreachable!("Choice prompts and labels are references")
        }
    }
}

fn main() {
    let directory = PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&directory).unwrap();
    let program_bytes = encode_program_artifact(&branch_call_choice_v1());
    let program = narrata_core::load_program(&program_bytes, &Default::default()).unwrap();
    let mut coordinator = SessionCoordinator::create(
        MemoryStore::new(),
        Arc::clone(&program),
        ExecutionId::from_u128(600),
        narrata_store::RefName::new("compat-session").unwrap(),
        BranchId::from_u128(600),
        InitialRecordingMode::Complete,
        1,
    )
    .unwrap();
    let say = coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(600)),
            Default::default(),
            2,
        )
        .unwrap();
    let interaction = say.state.pending().unwrap().interaction_id();
    let committed = coordinator
        .dispatch(
            CheckedRuntimeInput::advance(InputId::from_u128(601), interaction),
            Default::default(),
            3,
        )
        .unwrap();
    let DraftResult::AwaitChoice(choice) = &committed.result else {
        panic!("the corpus freezes the pending Choice");
    };
    let object = |id: &[u8; 32]| {
        coordinator
            .store()
            .get_object(ObjectId::from_bytes(*id))
            .unwrap()
            .unwrap()
    };
    let commit_object = object(committed.commit.as_bytes());
    let commit = CommitV1::decode(commit_object.payload()).unwrap();
    let receipt_object = object(committed.receipt.as_bytes());
    let snapshot_object = object(commit.snapshot.as_bytes());
    let checkpoint =
        CheckpointBundle::export(coordinator.store(), committed.commit, &BTreeSet::new())
            .unwrap()
            .to_bytes()
            .unwrap();
    let timeline = TimelineArchiveBundle::export(
        coordinator.store(),
        ExecutionId::from_u128(600),
        Some(coordinator.timeline()),
        &BTreeSet::new(),
    )
    .unwrap()
    .to_bytes()
    .unwrap();
    let content = format!("{}\n", story_content().to_json().unwrap());

    let mut artifacts = serde_json::Map::new();
    // Hex files are hashed by the bytes they carry, like the stage5-v0 corpus.
    let mut write = |name: &str, file: &[u8], hashed: &[u8]| {
        artifacts.insert(
            name.to_owned(),
            json!({ "sha256": hex::encode(sha256(hashed)) }),
        );
        std::fs::write(directory.join(name), file).unwrap();
    };
    for (name, bytes) in [
        ("program-v1.hex", program_bytes),
        ("snapshot-v1.hex", snapshot_object.bytes().to_vec()),
        ("receipt-v1.hex", receipt_object.bytes().to_vec()),
        ("commit-v1.hex", commit_object.bytes().to_vec()),
        ("checkpoint-bundle-v1.hex", checkpoint),
        ("timeline-archive-v1.hex", timeline),
    ] {
        write(
            name,
            format!("{}\n", hex::encode(&bytes)).as_bytes(),
            &bytes,
        );
    }
    write("content-pack.json", content.as_bytes(), content.as_bytes());
    let manifest = json!({
        "corpus_version": 1,
        "frozen_at_stage": 6,
        "artifacts": artifacts,
        "program_artifact_id": program.artifact_id().to_string(),
        "commit_id": committed.commit.to_string(),
        "receipt_id": committed.receipt.to_string(),
        "state_digest": narrata_core::snapshot::state_digest(&committed.state).to_string(),
        "content_pack": "content-pack.json",
        "expected_continuation": {
            "kind": "choice",
            "occurrence": choice.occurrence,
            "prompt": choice.prompt.as_ref().map(reference),
            "choices": choice.choices.iter().map(|item| json!({
                "id": item.id.to_string(),
                "label": reference(&item.label),
            })).collect::<Vec<_>>(),
            "select": ChoiceId::from_u128(1).to_string(),
        },
        "build_provenance": {
            "compiler": "narrata-testkit/0.1.0-alpha.1",
            "source": "generator:branch-call-choice-v1",
            "rust": "1.98",
            "locked_inputs": ["branch-call-choice-v1"],
        },
    });
    std::fs::write(
        directory.join("manifest.json"),
        format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
    )
    .unwrap();
}
