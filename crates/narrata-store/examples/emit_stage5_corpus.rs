#![allow(clippy::unwrap_used)]

use std::{collections::BTreeSet, sync::Arc};

use narrata_core::{
    ExecutionId, InputId, codec::sha256, program::encode_program_artifact,
    runtime::CheckedRuntimeInput,
};
use narrata_store::{
    BranchId, CheckpointBundle, InitialRecordingMode, MemoryStore, SaveStore, SessionCoordinator,
    TimelineArchiveBundle,
};
use narrata_testkit::generator::hello_v0;

fn main() {
    let program_bytes = encode_program_artifact(&hello_v0());
    let program = narrata_core::load_program(&program_bytes, &Default::default()).unwrap();
    let mut coordinator = SessionCoordinator::create(
        MemoryStore::new(),
        Arc::clone(&program),
        ExecutionId::from_u128(500),
        narrata_store::RefName::new("compat-session").unwrap(),
        BranchId::from_u128(500),
        InitialRecordingMode::Complete,
        1,
    )
    .unwrap();
    let committed = coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(500)),
            Default::default(),
            2,
        )
        .unwrap();
    let commit_object = coordinator
        .store()
        .get_object(narrata_core::ObjectId::from_bytes(
            *committed.commit.as_bytes(),
        ))
        .unwrap()
        .unwrap();
    let commit = narrata_store::CommitV1::decode(commit_object.payload()).unwrap();
    let receipt_object = coordinator
        .store()
        .get_object(narrata_core::ObjectId::from_bytes(
            *committed.receipt.as_bytes(),
        ))
        .unwrap()
        .unwrap();
    let snapshot_object = coordinator
        .store()
        .get_object(narrata_core::ObjectId::from_bytes(
            *commit.snapshot.as_bytes(),
        ))
        .unwrap()
        .unwrap();
    let checkpoint =
        CheckpointBundle::export(coordinator.store(), committed.commit, &BTreeSet::new())
            .unwrap()
            .to_bytes()
            .unwrap();
    let timeline = TimelineArchiveBundle::export(
        coordinator.store(),
        ExecutionId::from_u128(500),
        Some(coordinator.timeline()),
        &BTreeSet::new(),
    )
    .unwrap()
    .to_bytes()
    .unwrap();
    let fields = [
        ("program", program_bytes),
        ("snapshot", snapshot_object.bytes().to_vec()),
        ("receipt", receipt_object.bytes().to_vec()),
        ("commit", commit_object.bytes().to_vec()),
        ("checkpoint_bundle", checkpoint),
        ("timeline_archive", timeline),
    ];
    for (name, bytes) in fields {
        println!("{name}.sha256={}", hex::encode(sha256(&bytes)));
        println!("{name}.hex={}", hex::encode(bytes));
    }
    println!("artifact_id={}", program.artifact_id());
    println!("commit_id={}", committed.commit);
    println!("receipt_id={}", committed.receipt);
    println!(
        "state_digest={}",
        narrata_core::snapshot::state_digest(&committed.state)
    );
    println!("expected=say:Hello");
    println!("provenance.compiler=narrata-testkit/0.1.0-alpha.1");
    println!("provenance.source=generator:hello-v0");
}
