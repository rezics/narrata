#![allow(clippy::panic, clippy::unwrap_used)]

use std::{collections::BTreeSet, sync::Arc};

use narrata_core::{
    ExecutionId, InputId,
    program::{encode_program_artifact, load_program},
    runtime::CheckedRuntimeInput,
};
use narrata_store::{
    BranchId, BundleError, BundleLimits, CheckpointBundle, InitialRecordingMode, MemoryStore,
    RefName, SessionCoordinator,
};
use narrata_testkit::generator::branch_call_choice_v0;

fn bytes() -> Vec<u8> {
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
    let committed = coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap();
    CheckpointBundle::export(coordinator.store(), committed.commit, &BTreeSet::new())
        .unwrap()
        .to_bytes()
        .unwrap()
}

#[test]
fn truncation_hash_corruption_wrong_tag_and_limits_are_rejected() {
    let valid = bytes();
    for length in [0, 1, 7, 8, valid.len() / 2, valid.len() - 1] {
        assert!(CheckpointBundle::from_bytes(&valid[..length], Default::default()).is_err());
    }
    let mut corrupt = valid.clone();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 0x80;
    assert!(CheckpointBundle::from_bytes(&corrupt, Default::default()).is_err());
    assert!(matches!(
        narrata_store::TimelineArchiveBundle::from_bytes(&valid, Default::default()),
        Err(BundleError::WrongKind)
    ));
    assert!(matches!(
        CheckpointBundle::from_bytes(
            &valid,
            BundleLimits {
                max_total_bytes: 16,
                ..BundleLimits::default()
            }
        ),
        Err(BundleError::Limit("total bytes"))
    ));
}
