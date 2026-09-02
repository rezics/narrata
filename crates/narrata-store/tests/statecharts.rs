#![allow(clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;

use narrata_core::{
    EventTypeId, ExecutionId, InputId, ObjectId, Value, builtin_host_capabilities,
    program::{encode_program_artifact, load_program},
    runtime::{CheckedRuntimeInput, DraftResult},
};
use narrata_store::{
    BranchId, EffectClaimResult, InitialRecordingMode, LeaseId, MemoryStore, RefName, SaveStore,
    SessionCoordinator, TransitionReceiptV1,
};
use narrata_testkit::generator::statechart_parallel_history_v0;

#[test]
fn statechart_effects_commit_before_dispatch_and_rewind_exactly() {
    let program = load_program(
        &encode_program_artifact(&statechart_parallel_history_v0().unwrap()),
        &Default::default(),
    )
    .unwrap();
    let mut coordinator = SessionCoordinator::create_with_capabilities(
        MemoryStore::new(),
        Arc::clone(&program),
        ExecutionId::from_u128(700),
        RefName::new("statechart-session").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Complete,
        1,
        &builtin_host_capabilities().unwrap(),
    )
    .unwrap();

    let first = coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap();
    assert!(matches!(first.result, DraftResult::AwaitEffect(_)));
    assert!(
        coordinator
            .store()
            .get_object(ObjectId::from_bytes(*first.commit.as_bytes()))
            .unwrap()
            .is_some(),
        "the invoked Flow Effect must only be visible after its pending Commit"
    );
    coordinator
        .claim_pending_effect(LeaseId::from_u128(1), 3, 30)
        .unwrap();
    coordinator
        .complete_pending_effect(LeaseId::from_u128(1), Value::I64(42), 4)
        .unwrap();
    let second = coordinator
        .recover_pending_effect(Default::default(), 5)
        .unwrap()
        .unwrap();
    assert!(matches!(second.result, DraftResult::AwaitEffect(_)));
    assert!(matches!(
        coordinator
            .claim_pending_effect(LeaseId::from_u128(2), 6, 30)
            .unwrap(),
        EffectClaimResult::Claimed(_)
    ));
    coordinator
        .complete_pending_effect(LeaseId::from_u128(2), Value::I64(99), 7)
        .unwrap();
    let stable = coordinator
        .recover_pending_effect(Default::default(), 8)
        .unwrap()
        .unwrap();
    let DraftResult::StatechartStable(view) = &stable.result else {
        panic!("expected stable Statechart");
    };
    assert_eq!(view.active.len(), 2);
    let stable_commit = stable.commit;

    let pause = coordinator
        .dispatch(
            CheckedRuntimeInput::event(InputId::from_u128(4), EventTypeId::from_u128(3)),
            Default::default(),
            9,
        )
        .unwrap();
    let receipt_object = coordinator
        .store()
        .get_object(ObjectId::from_bytes(*pause.receipt.as_bytes()))
        .unwrap()
        .unwrap();
    let receipt = TransitionReceiptV1::decode(receipt_object.payload()).unwrap();
    assert_eq!(receipt.microstep_count, 1);
    assert_eq!(receipt.result_kind as u8, 4);

    coordinator.rewind_to(stable_commit, 10).unwrap();
    let replay = coordinator
        .dispatch(
            CheckedRuntimeInput::event(InputId::from_u128(4), EventTypeId::from_u128(3)),
            Default::default(),
            11,
        )
        .unwrap();
    assert!(replay.reused);
    assert_eq!(replay.commit, pause.commit);
    assert_eq!(replay.state, pause.state);
}
