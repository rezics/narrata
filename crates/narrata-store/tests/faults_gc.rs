#![allow(clippy::panic, clippy::unwrap_used)]

#[macro_use]
mod support;

use std::collections::BTreeSet;

use narrata_core::{ExecutionId, InputId, runtime::CheckedRuntimeInput};
use narrata_storage::{StorageBackend, testing::FaultInjecting};
use narrata_store::{
    BranchId, CheckpointBundle, CommitTransaction, InitialRecordingMode, Pin, RefKey, RefMutation,
    RefName, RetentionPolicy, SaveStore, SessionCoordinator, Store, layout,
};
use narrata_testkit::generator::branch_call_choice_v0;
use support::{Backend, assert_atomic, image, program};

fn coordinator<S: SaveStore>(store: S) -> SessionCoordinator<S> {
    SessionCoordinator::create(
        store,
        program(&branch_call_choice_v0()),
        ExecutionId::from_u128(1),
        RefName::new("session").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Complete,
        1,
    )
    .unwrap()
}

fn start<S: SaveStore>(coordinator: &mut SessionCoordinator<S>) -> narrata_core::CommitId {
    coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap()
        .commit
}

type Faulty<B> = SessionCoordinator<Store<FaultInjecting<<B as Backend>::Inner>>>;

fn store<B: StorageBackend>(
    coordinator: &mut SessionCoordinator<Store<FaultInjecting<B>>>,
) -> &mut Store<FaultInjecting<B>> {
    coordinator.store_mut()
}

fn dispatch_is_atomic_under_every_fault<B: Backend>() {
    let calls = assert_atomic(
        || coordinator(B::faulty()),
        store,
        |coordinator: &mut Faulty<B>| {
            coordinator.dispatch(
                CheckedRuntimeInput::start(InputId::from_u128(1)),
                Default::default(),
                2,
            )
        },
    );
    assert!(calls > 0);
}

fn archive_seal_is_atomic_under_every_fault<B: Backend>() {
    assert_atomic(
        || {
            let mut coordinator = coordinator(B::faulty());
            start(&mut coordinator);
            coordinator
        },
        store,
        |coordinator: &mut Faulty<B>| {
            coordinator.seal_complete_recording(RefName::new("sealed").unwrap(), 3)
        },
    );
}

fn bundle_import_is_atomic_under_every_fault<B: Backend>() {
    let mut source = coordinator(B::store());
    let commit = start(&mut source);
    let bundle = CheckpointBundle::export(source.store(), commit, &BTreeSet::new()).unwrap();
    let target = RefKey::save(
        RefName::new("import").unwrap(),
        RefName::new("slot").unwrap(),
    );
    assert_atomic(
        B::faulty,
        |store| store,
        |store| bundle.clone().import(store, target.clone(), None, 4),
    );
}

fn gc_sweep_is_atomic_under_every_fault<B: Backend>() {
    let calls = assert_atomic(
        || {
            let mut coordinator = coordinator(B::faulty());
            start(&mut coordinator);
            coordinator.delete_complete_recording(3).unwrap();
            coordinator
        },
        store,
        |coordinator: &mut Faulty<B>| coordinator.store_mut().collect(RetentionPolicy::default()),
    );
    assert!(calls > 0);
}

fn pin_and_ref_writes_are_atomic_under_every_fault<B: Backend>() {
    let setup = || {
        let mut coordinator = coordinator(B::faulty());
        let commit = start(&mut coordinator);
        (coordinator.into_store(), commit)
    };
    let (_, commit) = setup();
    let key = RefKey::save(
        RefName::new("player").unwrap(),
        RefName::new("one").unwrap(),
    );
    assert_atomic(
        setup,
        |(store, _)| store,
        |(store, _)| {
            store.commit(CommitTransaction {
                refs: vec![RefMutation {
                    key: key.clone(),
                    expected: None,
                    next: Some(commit),
                }],
                pins: vec![Pin {
                    owner: "host".to_owned(),
                    object: narrata_core::ObjectId::from_bytes(*commit.as_bytes()),
                    expires_at: Some(10),
                }],
                observed_at: 5,
                ..CommitTransaction::default()
            })
        },
    );
}

fn gc_dry_run_matches_sweep_and_keeps_root_closure<B: Backend>() {
    let mut coordinator = coordinator(B::store());
    start(&mut coordinator);
    let rooted = image(coordinator.store().backend());
    let dry = coordinator
        .store_mut()
        .collect(RetentionPolicy {
            now: 100,
            grace_seconds: 0,
            dry_run: true,
        })
        .unwrap();
    assert_eq!(image(coordinator.store().backend()), rooted);
    assert!(dry.removed.values().all(|kind| kind.objects == 0));

    coordinator.delete_complete_recording(3).unwrap();
    let policy = RetentionPolicy {
        now: 100,
        grace_seconds: 0,
        dry_run: true,
    };
    let before = image(coordinator.store().backend());
    let dry = coordinator.store_mut().collect(policy).unwrap();
    assert_eq!(image(coordinator.store().backend()), before);
    let sweep = coordinator
        .store_mut()
        .collect(RetentionPolicy {
            dry_run: false,
            ..policy
        })
        .unwrap();
    let removed = sweep.removed.values().map(|kind| kind.objects).sum::<u64>();
    assert!(removed > 0 && sweep.reachable > 0);
    assert_eq!(
        (dry.removed, dry.roots, dry.reachable),
        (sweep.removed, sweep.roots, sweep.reachable)
    );
    let after = image(coordinator.store().backend());
    assert_eq!(
        after.object_count() as u64,
        before.object_count() as u64 - removed
    );
    // Each stored object keeps its touch key, and nothing else is left of a deleted one.
    assert_eq!(after.space(layout::TOUCH).len(), after.object_count());
    assert!(coordinator.store().integrity_scan().unwrap().is_empty());
}

fn gc_keeps_objects_inside_the_grace_period<B: Backend>() {
    let mut coordinator = coordinator(B::store());
    start(&mut coordinator);
    coordinator.delete_complete_recording(3).unwrap();
    let before = image(coordinator.store().backend());
    let report = coordinator
        .store_mut()
        .collect(RetentionPolicy {
            now: 4,
            grace_seconds: 10,
            dry_run: false,
        })
        .unwrap();
    assert!(report.removed.values().all(|kind| kind.objects == 0));
    assert_eq!(image(coordinator.store().backend()).objects, before.objects);
}

backend_tests!(
    dispatch_is_atomic_under_every_fault,
    archive_seal_is_atomic_under_every_fault,
    bundle_import_is_atomic_under_every_fault,
    gc_sweep_is_atomic_under_every_fault,
    pin_and_ref_writes_are_atomic_under_every_fault,
    gc_dry_run_matches_sweep_and_keeps_root_closure,
    gc_keeps_objects_inside_the_grace_period,
);
