//! Behaviour of the save engine over any backend: layout, preconditions, unknown outcomes,
//! batches split by backend limits, and races between handles (ADR 0014).

#![allow(clippy::panic, clippy::unwrap_used)]

#[macro_use]
mod support;

use std::{collections::BTreeSet, sync::Arc};

use narrata_core::{
    CheckedProgram, CommitId, ExecutionId, InputId, ObjectId, runtime::CheckedRuntimeInput,
};
use narrata_storage::{
    Applied, Batch, Capabilities, Expect, KeyPage, KeySpace, KeyValue, Limits, ObjectDigest,
    ObjectPage, StorageBackend, StorageError,
    testing::{Counting, Fault, FaultInjecting, FaultPlan, Primitive, Trigger},
};
use narrata_store::{
    BranchId, CheckpointBundle, CommitTransaction, InitialRecordingMode, RefKey, RefMutation,
    RefName, RetentionPolicy, SaveStore, SessionCoordinator, Store, StoreError, layout,
    load_commit, timeline_branch,
};
use support::{Backend, image, looping_program, refs};

fn advance<S: SaveStore>(session: &mut SessionCoordinator<S>, turn: u64) -> CommitId {
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
        .unwrap()
        .commit
}

fn session<S: SaveStore>(
    store: S,
    program: &Arc<CheckedProgram>,
    execution: u128,
    turns: u64,
) -> SessionCoordinator<S> {
    let mut session = SessionCoordinator::create(
        store,
        Arc::clone(program),
        ExecutionId::from_u128(execution),
        RefName::new("history").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Standard,
        1,
    )
    .unwrap();
    for turn in 1..=turns {
        advance(&mut session, turn);
    }
    session
}

fn slot(name: &str) -> RefKey {
    RefKey::save(RefName::new("player").unwrap(), RefName::new(name).unwrap())
}

fn set(
    key: &RefKey,
    expected: Option<narrata_store::RefRevision>,
    next: Option<CommitId>,
) -> CommitTransaction {
    CommitTransaction {
        refs: vec![RefMutation {
            key: key.clone(),
            expected,
            next,
        }],
        ..CommitTransaction::default()
    }
}

fn io() -> Fault {
    Fault::Before(StorageError::Io("injected".to_owned()))
}

fn format_error<T: std::fmt::Debug>(result: Result<T, StoreError>) {
    assert!(
        matches!(result, Err(StoreError::Storage(StorageError::Format(_)))),
        "expected a format error, got {result:?}"
    );
}

fn open_refuses_foreign_data_and_other_layouts<B: Backend>() {
    let (mut foreign, observer, _guard) = B::pair();
    foreign
        .apply(&Batch::new().put(KeySpace::new(2), b"x", b"y", Expect::Absent))
        .unwrap();
    let before = image(&observer);
    format_error(Store::open(foreign).map(|_| ()));
    assert_eq!(
        image(&observer),
        before,
        "a refused backend is left untouched"
    );

    let mut newer = B::create();
    // The layout marker `{0: 2}` in canonical CBOR: layout version 2.
    newer
        .apply(&Batch::new().put(layout::META, b"layout", [0xa1, 0x00, 0x02], Expect::Absent))
        .unwrap();
    format_error(Store::open(newer).map(|_| ()));

    let store = Store::open(B::create()).unwrap();
    let opened = image(store.backend());
    assert_eq!(
        opened.keys.len(),
        1,
        "a new store holds only its layout marker"
    );
    let store = Store::open(store.into_backend()).unwrap();
    assert_eq!(image(store.backend()), opened);
}

fn tampered_values_and_objects_are_refused<B: Backend>() {
    let program = looping_program();
    let mut store = session(B::store(), &program, 1, 1).into_store();
    let head = refs(&store)[0].1.commit;
    let key = slot("tampered");
    store.commit(set(&key, None, Some(head))).unwrap();
    let entry = store
        .backend()
        .scan_keys(layout::REFS, b"", None, 100)
        .unwrap()
        .entries
        .into_iter()
        .find(|entry| entry.key.ends_with(b"tampered"))
        .unwrap();
    // The same Commit ID with a trailing byte: decodable once, but not canonical.
    let mut value = entry.value.clone();
    value.push(0);
    store
        .backend_mut()
        .apply(&Batch::new().put(layout::REFS, entry.key, value, Expect::Any))
        .unwrap();
    assert!(matches!(
        store.read_ref(&key),
        Err(StoreError::CorruptStore(_))
    ));

    let snapshot = load_commit(&store, head, &program).unwrap().commit.snapshot;
    let wrong = ObjectDigest::from_bytes([7; 32]);
    let bytes = store
        .get_object(ObjectId::from_bytes(*snapshot.as_bytes()))
        .unwrap()
        .unwrap()
        .bytes()
        .to_vec();
    store
        .backend_mut()
        .apply(&Batch::new().put_object(wrong, bytes))
        .unwrap();
    assert!(matches!(
        store.get_object(ObjectId::from_bytes([7; 32])),
        Err(StoreError::Corrupt(..))
    ));
}

/// A revision names one write, not one value: setting a Ref back to an earlier Commit does not
/// revive the earlier revision.
fn ref_revision_is_not_fooled_by_a_restored_value<B: Backend>() {
    let program = looping_program();
    let mut session = session(B::store(), &program, 1, 0);
    let genesis = session.timeline().cursor;
    let first = advance(&mut session, 1);
    let mut store = session.into_store();
    let key = slot("aba");
    let at = |outcome: narrata_store::CommitOutcome| outcome.refs[&key].unwrap().revision;
    let r1 = at(store.commit(set(&key, None, Some(genesis))).unwrap());
    let r2 = at(store.commit(set(&key, Some(r1), Some(first))).unwrap());
    let r3 = at(store.commit(set(&key, Some(r2), Some(genesis))).unwrap());
    assert!(r1 != r3 && r2 != r3);
    let Err(StoreError::RefConflict(conflict)) = store.commit(set(&key, Some(r1), Some(first)))
    else {
        panic!("a stale revision must conflict");
    };
    assert_eq!(conflict.expected, Some(r1));
    assert_eq!(conflict.actual.unwrap().revision, r3);
    assert_eq!(conflict.actual.unwrap().commit, genesis);
}

/// A batch that applied but reported an I/O error is recognised as applied, with the revision
/// the backend gave it; one that did not apply is reported as failed.
fn unknown_outcomes_are_reconciled<B: Backend>() {
    let program = looping_program();
    let mut store = session(B::faulty(), &program, 1, 1).into_store();
    let head = refs(&store)[0].1.commit;

    let key = slot("after");
    store
        .backend_mut()
        .set_plan(FaultPlan::new().at(Trigger::Nth(Primitive::Apply, 1), Fault::After));
    let outcome = store.commit(set(&key, None, Some(head))).unwrap();
    store.backend_mut().set_plan(FaultPlan::new());
    assert_eq!(outcome.refs[&key], store.read_ref(&key).unwrap());

    let key = slot("before");
    store
        .backend_mut()
        .set_plan(FaultPlan::new().at(Trigger::Nth(Primitive::Apply, 1), io()));
    assert!(matches!(
        store.commit(set(&key, None, Some(head))),
        Err(StoreError::Io(_))
    ));
    store.backend_mut().set_plan(FaultPlan::new());
    assert_eq!(store.read_ref(&key).unwrap(), None);
}

fn import(bundle: &CheckpointBundle, store: &mut impl SaveStore) -> Result<(), String> {
    for target in [
        timeline_branch(ExecutionId::from_u128(1), BranchId::from_u128(1)),
        RefKey::active(RefName::new("history").unwrap()).unwrap(),
    ] {
        bundle
            .clone()
            .import(store, target, None, 50)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

const SMALL: Limits = Limits {
    max_batch_ops: 12,
    ..Limits::DEFAULT
};

/// A history larger than one batch is written referents first, and its roots last.
fn imports_over_batch_limits_split_and_stay_closed<B: Backend>() {
    let program = looping_program();
    let source = session(B::store(), &program, 1, 30);
    let head = source.timeline().cursor;
    let bundle = CheckpointBundle::export(source.store(), head, &BTreeSet::new()).unwrap();

    let mut store = Store::open(Counting::new(B::with_limits(SMALL))).unwrap();
    import(&bundle, &mut store).unwrap();
    assert!(store.backend().counts().apply > 10);
    assert!(store.integrity_scan().unwrap().is_empty());
    let mut restored = SessionCoordinator::open(
        store,
        Arc::clone(&program),
        ExecutionId::from_u128(1),
        RefName::new("history").unwrap(),
        BranchId::from_u128(1),
    )
    .unwrap();
    assert_eq!(restored.timeline().cursor, head);
    advance(&mut restored, 31);

    // Interrupted after some batches: no root moved, every stored object's references are
    // stored, the import can run again, and GC removes what an abandoned import left.
    let mut store =
        Store::open(FaultInjecting::new(B::with_limits(SMALL), FaultPlan::new())).unwrap();
    store
        .backend_mut()
        .set_plan(FaultPlan::new().at(Trigger::Nth(Primitive::Apply, 5), io()));
    assert!(import(&bundle, &mut store).is_err());
    store.backend_mut().set_plan(FaultPlan::new());
    assert!(refs(&store).is_empty());
    assert!(!image(store.backend().inner()).objects.is_empty());
    assert!(store.integrity_scan().unwrap().is_empty());
    store.collect(RetentionPolicy::default()).unwrap();
    assert!(image(store.backend().inner()).objects.is_empty());
    import(&bundle, &mut store).unwrap();
    assert_eq!(refs(&store).len(), 2);
    assert!(store.integrity_scan().unwrap().is_empty());
}

/// Runs a hook just before the wrapped backend's next batch, to interleave another handle.
struct Interleave<B> {
    inner: B,
    hook: Option<Box<dyn FnOnce()>>,
}

impl<B: StorageBackend> StorageBackend for Interleave<B> {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    fn get_objects(
        &self,
        digests: &[ObjectDigest],
    ) -> Result<Vec<Option<Arc<[u8]>>>, StorageError> {
        self.inner.get_objects(digests)
    }

    fn scan_objects(
        &self,
        after: Option<&ObjectDigest>,
        limit: u32,
    ) -> Result<ObjectPage, StorageError> {
        self.inner.scan_objects(after, limit)
    }

    fn read_key(&self, space: KeySpace, key: &[u8]) -> Result<Option<KeyValue>, StorageError> {
        self.inner.read_key(space, key)
    }

    fn scan_keys(
        &self,
        space: KeySpace,
        prefix: &[u8],
        after: Option<&[u8]>,
        limit: u32,
    ) -> Result<KeyPage, StorageError> {
        self.inner.scan_keys(space, prefix, after, limit)
    }

    fn apply(&mut self, batch: &Batch) -> Result<Applied, StorageError> {
        if let Some(hook) = self.hook.take() {
            hook();
        }
        self.inner.apply(batch)
    }
}

/// Two handles to one store holding a session of execution 1 and, unreachable, the history of
/// execution 2 up to `garbage`.
struct Shared<B: Backend> {
    first: Store<B::Inner>,
    second: Store<B::Inner>,
    garbage: CommitId,
    _guard: Option<tempfile::TempDir>,
}

fn with_garbage<B: Backend>() -> Shared<B> {
    let (first, second, guard) = B::pair();
    let program = looping_program();
    let mut store = session(Store::open(first).unwrap(), &program, 1, 2).into_store();
    let other = session(B::store(), &program, 2, 2);
    let garbage = other.timeline().cursor;
    let bundle = CheckpointBundle::export(other.store(), garbage, &BTreeSet::new()).unwrap();
    let key = slot("dropped");
    let value = bundle.import(&mut store, key.clone(), None, 10).unwrap();
    store.commit(set(&key, Some(value.revision), None)).unwrap();
    Shared {
        first: store,
        second: Store::open(second).unwrap(),
        garbage,
        _guard: guard,
    }
}

fn reachable(store: &impl SaveStore, commit: CommitId) -> bool {
    store.integrity_scan().unwrap().is_empty()
        && store
            .get_object(ObjectId::from_bytes(*commit.as_bytes()))
            .unwrap()
            .is_some()
}

/// A write that adds a root between GC's mark and its deletion makes GC mark again.
fn gc_marks_again_after_a_concurrent_root<B: Backend>() {
    let Shared {
        first: collector,
        second: mut writer,
        garbage,
        _guard,
    } = with_garbage::<B>();
    let key = slot("rescued");
    let rescue = set(&key, None, Some(garbage));
    let mut collector = Store::open(Interleave {
        inner: collector.into_backend(),
        hook: Some(Box::new(move || {
            writer.commit(rescue).unwrap();
        })),
    })
    .unwrap();
    collector.collect(RetentionPolicy::default()).unwrap();
    assert_eq!(collector.read_ref(&key).unwrap().unwrap().commit, garbage);
    assert!(reachable(&collector, garbage));
    assert!(
        load_commit(&collector, garbage, &looping_program()).is_ok(),
        "the rescued history is whole"
    );
}

/// A write planned against objects that GC deletes before the write applies is planned again,
/// and then fails instead of publishing a Ref to a missing Commit.
fn write_plans_again_after_a_concurrent_sweep<B: Backend>() {
    let Shared {
        first: writer,
        second: mut collector,
        garbage,
        _guard,
    } = with_garbage::<B>();
    let mut writer = Store::open(Interleave {
        inner: writer.into_backend(),
        hook: Some(Box::new(move || {
            let report = collector.collect(RetentionPolicy::default()).unwrap();
            assert!(report.removed.values().any(|kind| kind.objects > 0));
        })),
    })
    .unwrap();
    let key = slot("too-late");
    let result = writer.commit(set(&key, None, Some(garbage)));
    assert!(
        matches!(result, Err(StoreError::MissingObject(_))),
        "got {result:?}"
    );
    assert_eq!(writer.read_ref(&key).unwrap(), None);
    assert!(writer.integrity_scan().unwrap().is_empty());
}

fn two_handles_cannot_silently_overwrite_a_slot<B: Backend>() {
    let (first, second, _guard) = B::pair();
    let program = looping_program();
    let mut first = session(Store::open(first).unwrap(), &program, 1, 1).into_store();
    let mut second = Store::open(second).unwrap();
    let head = refs(&first)[0].1.commit;
    let key = slot("shared");
    first.commit(set(&key, None, Some(head))).unwrap();
    assert!(matches!(
        second.commit(set(&key, None, Some(head))),
        Err(StoreError::RefConflict(_))
    ));
    assert_eq!(
        second.read_ref(&key).unwrap(),
        first.read_ref(&key).unwrap()
    );
}

/// Two handles racing compare-and-set on one Ref never both win from the same revision.
fn racing_handles_win_each_revision_once<B: Backend>() {
    let (first, second, _guard) = B::pair();
    let program = looping_program();
    let store = session(Store::open(first).unwrap(), &program, 1, 1).into_store();
    let head = refs(&store)[0].1.commit;
    let key = slot("race");
    let threads = [store.into_backend(), second].map(|backend| {
        let key = key.clone();
        std::thread::spawn(move || {
            let mut store = Store::open(backend).unwrap();
            let mut won = Vec::new();
            for _ in 0..20 {
                let current = store.read_ref(&key).unwrap().map(|value| value.revision);
                match store.commit(set(&key, current, Some(head))) {
                    Ok(outcome) => won.push((current, outcome.refs[&key].unwrap().revision)),
                    Err(StoreError::RefConflict(_) | StoreError::Busy) => {}
                    Err(error) => panic!("unexpected {error:?}"),
                }
            }
            won
        })
    });
    let won = threads
        .map(|thread| thread.join().unwrap())
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let predecessors = won.iter().map(|(from, _)| *from).collect::<BTreeSet<_>>();
    let revisions = won.iter().map(|(_, to)| *to).collect::<BTreeSet<_>>();
    assert!(
        won.len() >= 20,
        "each handle's first read or a later one wins: {won:?}"
    );
    assert_eq!(
        predecessors.len(),
        won.len(),
        "two writers won from one revision"
    );
    assert_eq!(revisions.len(), won.len());
}

backend_tests!(
    open_refuses_foreign_data_and_other_layouts,
    tampered_values_and_objects_are_refused,
    ref_revision_is_not_fooled_by_a_restored_value,
    unknown_outcomes_are_reconciled,
    imports_over_batch_limits_split_and_stay_closed,
    gc_marks_again_after_a_concurrent_root,
    write_plans_again_after_a_concurrent_sweep,
    two_handles_cannot_silently_overwrite_a_slot,
    racing_handles_win_each_revision_once,
);
