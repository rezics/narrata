#![allow(clippy::panic, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc};

use narrata_core::{
    ExecutionId, InputId, ObjectId,
    program::{encode_program_artifact, load_program},
    runtime::CheckedRuntimeInput,
};
use narrata_store::{
    BranchId, InitialRecordingMode, MemoryStore, RefName, SaveStore, SessionCoordinator,
};
use narrata_testkit::generator::branch_call_choice_v0;

#[test]
fn initial_and_normal_commit_bytes_match_golden_vectors() {
    let program = load_program(
        &encode_program_artifact(&branch_call_choice_v0()),
        &Default::default(),
    )
    .unwrap();
    let mut coordinator = SessionCoordinator::create(
        MemoryStore::new(),
        Arc::clone(&program),
        ExecutionId::from_u128(1),
        RefName::new("session").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Standard,
        1,
    )
    .unwrap();
    let genesis = coordinator.timeline().cursor;
    let normal = coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap()
        .commit;
    for (fixture, id) in [
        ("commit-genesis-v1.cbor.hex", genesis),
        ("commit-normal-v1.cbor.hex", normal),
    ] {
        let expected = std::fs::read_to_string(fixture_path(fixture)).unwrap();
        let object = coordinator
            .store()
            .get_object(ObjectId::from_bytes(*id.as_bytes()))
            .unwrap()
            .unwrap();
        assert_eq!(hex::encode(object.bytes()), expected.trim());
    }
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/codec")
        .join(name)
}
