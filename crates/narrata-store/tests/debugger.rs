#![allow(clippy::panic, clippy::unwrap_used)]

#[macro_use]
mod support;

use narrata_core::{
    ExecutionId, InputId,
    program::{encode_program_artifact, load_program},
    runtime::CheckedRuntimeInput,
    snapshot::state_digest,
};
use narrata_store::{
    BranchId, InitialRecordingMode, RefName, SessionCoordinator, inspect_timeline,
    verify_receipt_replay,
};
use narrata_testkit::generator::hello_v0;
use support::Backend;

fn debugger_views_complete_timeline_and_replays_receipt<B: Backend>() {
    let program = load_program(&encode_program_artifact(&hello_v0()), &Default::default()).unwrap();
    let execution = ExecutionId::from_u128(501);
    let mut coordinator = SessionCoordinator::create(
        B::store(),
        program.clone(),
        execution,
        RefName::new("debug-session").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Complete,
        1,
    )
    .unwrap();
    let input = CheckedRuntimeInput::start(InputId::from_u128(1));
    let committed = coordinator
        .dispatch(input.clone(), Default::default(), 2)
        .unwrap();

    let timeline = inspect_timeline(coordinator.store(), execution).unwrap();
    assert_eq!(timeline.nodes.len(), 2);
    assert_eq!(timeline.nodes[0].cause, "genesis");
    assert_eq!(timeline.nodes[1].cause, "transition");
    assert!(timeline.coverage.is_some());
    assert_eq!(timeline.refs.len(), 2);

    let verified = verify_receipt_replay(
        coordinator.store(),
        committed.receipt,
        program,
        input,
        Default::default(),
    )
    .unwrap();
    assert_eq!(verified.receipt, committed.receipt);
    assert_eq!(verified.next_state, state_digest(&committed.state));
}

backend_tests!(debugger_views_complete_timeline_and_replays_receipt,);
