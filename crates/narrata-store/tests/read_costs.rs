//! Loading and committing read the same backend data whatever the history depth (ADR 0014).

#![allow(clippy::panic, clippy::unwrap_used)]

#[macro_use]
mod support;

use std::sync::Arc;

use narrata_core::{CheckedProgram, ExecutionId, InputId, runtime::CheckedRuntimeInput};
use narrata_storage::{
    StorageBackend,
    testing::{Counting, Counts},
};
use narrata_store::{
    BranchId, InitialRecordingMode, RefName, SaveStore, SessionCoordinator, Store, load_commit,
};
use support::{Backend, looping_program};

fn advance<S: SaveStore>(session: &mut SessionCoordinator<S>, turn: u64) {
    let input = session.state().pending().map_or_else(
        || CheckedRuntimeInput::start(InputId::from_u128(u128::from(turn))),
        |pending| {
            CheckedRuntimeInput::advance(
                InputId::from_u128(u128::from(turn)),
                pending.interaction_id(),
            )
        },
    );
    session
        .dispatch(input, Default::default(), turn + 1)
        .unwrap();
}

fn open<S: SaveStore>(store: S, program: &Arc<CheckedProgram>) -> SessionCoordinator<S> {
    SessionCoordinator::open(
        store,
        Arc::clone(program),
        ExecutionId::from_u128(1),
        RefName::new("history").unwrap(),
        BranchId::from_u128(1),
    )
    .unwrap()
}

/// What loading the head, opening the session and committing one more turn each read, after
/// building a history of `depth` commits.
fn costs<B: StorageBackend>(backend: B, depth: u64) -> [Counts; 3] {
    let program = looping_program();
    let mut session = SessionCoordinator::create(
        Store::open(Counting::new(backend)).unwrap(),
        Arc::clone(&program),
        ExecutionId::from_u128(1),
        RefName::new("history").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Standard,
        1,
    )
    .unwrap();
    for turn in 1..=depth {
        advance(&mut session, turn);
    }
    let head = session.timeline().cursor;
    let store = session.into_store();

    store.backend().take_counts();
    let loaded = load_commit(&store, head, &program).unwrap();
    assert_eq!(loaded.commit.turn.0, depth);
    let load = store.backend().take_counts();

    let mut session = open(store, &program);
    let open = session.store().backend().take_counts();

    advance(&mut session, depth + 1);
    let commit = session.store().backend().take_counts();
    [load, open, commit]
}

fn reads_do_not_grow_with_history_depth<B: Backend>() {
    let shallow = costs(B::create(), 10);
    let deep = costs(B::create(), 1_000);
    assert_eq!(shallow, deep);
    for counts in deep {
        assert_eq!(counts.scan_objects, 0);
        assert_eq!(counts.objects_scanned, 0);
    }
    let [load, open, commit] = deep;
    assert_eq!(load.apply, 0);
    assert_eq!(open.apply, 0);
    assert_eq!(commit.apply, 1);
}

backend_tests!(reads_do_not_grow_with_history_depth);
