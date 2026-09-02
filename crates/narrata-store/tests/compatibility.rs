#![allow(clippy::panic, clippy::unwrap_used)]

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
};

use narrata_core::{
    CommitId, InputId, ObjectId, ReceiptId,
    codec::{ObjectKind, sha256},
    program::load_program,
    runtime::{
        CheckedRuntimeInput, DraftResult, PendingInteractionV0, SliceBudget, SliceOutcome,
        begin_transition_with_parent_commit,
    },
    snapshot::restore_snapshot,
};
use narrata_store::{
    BundleLimits, CheckedObject, CheckpointBundle, CommitV1, MemoryStore, RefKey, RefName,
    TimelineArchiveBundle, TransitionReceiptV1, load_commit,
};
use serde::Deserialize;

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

#[test]
fn frozen_stage5_corpus_loads_imports_and_continues() {
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
        Some(PendingInteractionV0::Say { text, .. }) if text.as_ref() == manifest.expected_continuation.text
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
    let mut store = MemoryStore::new();
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
