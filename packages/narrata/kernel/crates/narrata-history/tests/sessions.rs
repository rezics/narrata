//! Sessions of a registered domain over the history engine, on every backend (ADR 0015).

#![allow(clippy::panic, clippy::unwrap_used)]

#[macro_use]
mod support;

use std::collections::BTreeSet;

use narrata_history::{
    AdvanceError, AuditError, BundleError, BundleLimits, Commit, ContainerError, Domain,
    DomainKinds, History, HistoryError, Object, RefKey, RefMutation, Session, Transaction,
    testing::{COUNTER_COMMIT_KIND, COUNTER_STATE_KIND, Counter, Overflow},
};
use narrata_storage::testing::Counting;
use support::{COUNTER, Fixture, chain, heads, name, never, open, step};

on_both_backends!(
    sessions_advance_reuse_and_branch,
    children_come_back_in_pages,
    save_slots_load_without_replay,
    loading_reads_do_not_grow_with_depth,
    a_racing_step_that_disagrees_is_nondeterministic,
    checkpoints_import_checked,
    imports_that_contradict_recorded_transitions_are_refused,
    loading_checks_states_against_the_artifact,
    verify_path_replays_on_request,
);

fn sessions_advance_reuse_and_branch<F: Fixture>() {
    let fixture = F::new();
    let mut history = open(&fixture);
    let (mut session, root) =
        Session::create(&mut history, COUNTER, name("player"), &0, 1).unwrap();
    assert_eq!((root.state, root.header.depth), (0, 0));
    let main = session.branch().unwrap();

    let one = session
        .advance(&mut history, root.commit, &1, step, 2)
        .unwrap();
    assert_eq!((one.state, one.depth, one.reused), (1, 1, false));
    let three = session
        .advance(&mut history, one.commit, &2, step, 3)
        .unwrap();
    assert_eq!(session.branch(), Some(main));
    assert_eq!(
        heads(&history, "player").into_values().collect::<Vec<_>>(),
        [three.commit]
    );

    // Back at the root, the same input finds its recorded transition and the step does not run.
    let loaded = session
        .checkout(&mut history, three.commit, root.commit, 4)
        .unwrap();
    assert_eq!(loaded.state, 0);
    let again = session
        .advance(&mut history, root.commit, &1, never, 5)
        .unwrap();
    assert_eq!((again.commit, again.reused), (one.commit, true));

    // A new input off the head of the selected branch starts a branch at the new commit.
    let six = session
        .advance(&mut history, one.commit, &5, step, 6)
        .unwrap();
    let fork = session.branch().unwrap();
    assert_ne!(fork, main);
    assert_eq!(
        heads(&history, "player"),
        [(main, three.commit), (fork, six.commit)].into()
    );

    // Checking out a branch head selects that branch, and advancing moves it.
    session
        .checkout(&mut history, six.commit, three.commit, 7)
        .unwrap();
    assert_eq!(session.branch(), Some(main));
    let four = session
        .advance(&mut history, three.commit, &1, step, 8)
        .unwrap();
    assert_eq!(
        heads(&history, "player"),
        [(main, four.commit), (fork, six.commit)].into()
    );

    // The step's refusal is reported as the caller's, and nothing moves.
    assert_eq!(
        session.advance(&mut history, four.commit, &1_000, step, 9),
        Err(AdvanceError::Step(Overflow))
    );
    // So is a head the session no longer has.
    assert!(matches!(
        session.advance(&mut history, three.commit, &1, step, 9),
        Err(AdvanceError::History(HistoryError::HeadMoved { .. }))
    ));
    // Inputs the artifact refuses never reach the store.
    assert!(matches!(
        session.advance(&mut history, four.commit, &0, step, 9),
        Err(AdvanceError::History(HistoryError::Corrupt(..)))
    ));

    drop(history);
    let history = open(&fixture);
    let (reopened, loaded) = Session::open(&history, COUNTER, name("player")).unwrap();
    assert_eq!(
        (reopened.head(), reopened.branch()),
        (four.commit, Some(main))
    );
    assert_eq!((loaded.state, loaded.header.depth), (4, 3));
    // A session of another artifact cannot open these commits.
    assert!(matches!(
        Session::open(&history, Counter::new(10), name("player")),
        Err(HistoryError::ArtifactMismatch { .. })
    ));
}

fn children_come_back_in_pages<F: Fixture>() {
    let fixture = F::new();
    let mut history = open(&fixture);
    let (mut session, root) =
        Session::create(&mut history, COUNTER, name("player"), &0, 1).unwrap();
    let mut children = BTreeSet::new();
    for increment in 1..=5 {
        let head = session.head();
        session
            .checkout(&mut history, head, root.commit, 2)
            .unwrap();
        children.insert(
            session
                .advance(&mut history, root.commit, &increment, step, 3)
                .unwrap()
                .commit,
        );
    }
    let mut paged = Vec::new();
    let mut after = None;
    loop {
        let page = history.children(root.commit, after.as_ref(), 2).unwrap();
        assert!(page.items.len() <= 2);
        after = page.items.last().copied();
        paged.extend(page.items);
        if !page.more {
            break;
        }
    }
    assert_eq!(paged, children.into_iter().collect::<Vec<_>>());
    let grandchildren = history.children(paged[0], None, 10).unwrap();
    assert!(grandchildren.items.is_empty() && !grandchildren.more);
}

fn save_slots_load_without_replay<F: Fixture>() {
    let fixture = F::new();
    let mut history = open(&fixture);
    let (mut session, commits) = chain(&mut history, "player", &[1, 2, 3]);
    let slot = RefKey::save(name("player"), name("slot-1"));
    let saved = session.save(&mut history, slot.clone(), None, 5).unwrap();
    assert_eq!(saved.commit, commits[3]);
    // A slot is replaced only at the revision the caller saw.
    assert!(matches!(
        session.save(&mut history, slot.clone(), None, 6),
        Err(HistoryError::RefConflict(_))
    ));

    session
        .checkout(&mut history, commits[3], commits[1], 6)
        .unwrap();
    let loaded = session.load_save(&mut history, &slot, 7).unwrap();
    assert_eq!((loaded.commit, loaded.state), (commits[3], 6));
    assert_eq!(session.head(), commits[3]);
    let resaved = session
        .save(&mut history, slot.clone(), Some(saved.revision), 8)
        .unwrap();
    assert!(resaved.revision > saved.revision);
    assert!(matches!(
        session.load_save(&mut history, &RefKey::save(name("player"), name("none")), 9),
        Err(HistoryError::MissingRef(_))
    ));
}

/// Loading reads one commit and one state, whatever lies behind them.
fn loading_reads_do_not_grow_with_depth<F: Fixture>() {
    let fixture = F::new();
    let mut history = History::open(
        Counting::new(fixture.backend()),
        DomainKinds::<Counter>::new(),
    )
    .unwrap();
    let increments = vec![1; 64];
    let (mut session, commits) = chain(&mut history, "player", &increments);
    let shallow_slot = RefKey::save(name("player"), name("shallow"));
    let deep_slot = RefKey::save(name("player"), name("deep"));
    session
        .save(&mut history, deep_slot.clone(), None, 70)
        .unwrap();
    session
        .checkout(&mut history, commits[64], commits[1], 71)
        .unwrap();
    session
        .save(&mut history, shallow_slot.clone(), None, 72)
        .unwrap();

    history.backend().take_counts();
    let shallow = history.load(&COUNTER, commits[1]).unwrap();
    let shallow_counts = history.backend().take_counts();
    let deep = history.load(&COUNTER, commits[64]).unwrap();
    let deep_counts = history.backend().take_counts();
    assert_eq!((shallow.state, deep.state), (1, 64));
    assert_eq!(shallow_counts, deep_counts);
    assert_eq!(deep_counts.objects_returned, 2);
    assert_eq!(deep_counts.scan_keys + deep_counts.scan_objects, 0);

    // Loading a save reads the slot, the cursor and the session's branches, never the path.
    session.load_save(&mut history, &deep_slot, 73).unwrap();
    let deep_counts = history.backend().take_counts();
    session.load_save(&mut history, &shallow_slot, 74).unwrap();
    let shallow_counts = history.backend().take_counts();
    assert_eq!(shallow_counts, deep_counts);
}

/// Two handles on one store: while the second runs its step, the first records the same
/// transition with another result.
fn a_racing_step_that_disagrees_is_nondeterministic<F: Fixture>() {
    let fixture = F::new();
    let mut first = open(&fixture);
    let mut second = open(&fixture);
    let (mut racer, root) = Session::create(&mut first, COUNTER, name("player"), &0, 1).unwrap();
    let (mut session, _) = Session::open(&second, COUNTER, name("player")).unwrap();
    let error = session
        .advance(
            &mut second,
            root.commit,
            &1,
            |_, state, input| {
                racer
                    .advance(&mut first, root.commit, input, step, 2)
                    .unwrap();
                Ok::<_, Overflow>(state + input + 1)
            },
            3,
        )
        .unwrap_err();
    let AdvanceError::History(HistoryError::Nondeterministic(found)) = error else {
        panic!("expected nondeterminism, got {error:?}");
    };
    assert_eq!((found.parent, found.recorded), (root.commit, racer.head()));
    assert_ne!(found.proposed, found.recorded);

    // The recorded transition stands: a fresh look at the session reuses it.
    let (mut session, loaded) = Session::open(&second, COUNTER, name("player")).unwrap();
    assert_eq!(loaded.state, 1);
    session
        .checkout(&mut second, racer.head(), root.commit, 4)
        .unwrap();
    let again = session
        .advance(&mut second, root.commit, &1, never, 5)
        .unwrap();
    assert_eq!(again.commit, racer.head());
}

fn checkpoints_import_checked<F: Fixture>() {
    let source_store = F::new();
    let mut source = open(&source_store);
    let (_, commits) = chain(&mut source, "player", &[1, 2, 3]);
    let bundle = source.export(commits[2], &BTreeSet::new()).unwrap();
    // The root's closure: three commits, three states and two inputs.
    assert_eq!(bundle.manifest.objects.len(), 8);
    let bytes = bundle.to_bytes().unwrap();

    // Any flipped byte is refused before anything is written.
    let target_store = F::new();
    let mut target = open(&target_store);
    let slot = RefKey::save(name("imported"), name("slot"));
    let limits = BundleLimits::default();
    for index in 0..bytes.len() {
        let mut tampered = bytes.clone();
        tampered[index] ^= 0x01;
        assert!(
            target
                .import(&COUNTER, &tampered, limits, slot.clone(), None, 9)
                .is_err(),
            "flipping byte {index} went unnoticed"
        );
    }
    assert!(target.is_empty_except_meta().unwrap());
    // So is a bundle cut short, or one larger than the receiver allows.
    assert!(matches!(
        target.import(
            &COUNTER,
            &bytes[..bytes.len() - 1],
            limits,
            slot.clone(),
            None,
            9
        ),
        Err(BundleError::Container(ContainerError::Truncated))
    ));
    let small = BundleLimits {
        max_objects: 4,
        ..limits
    };
    assert!(matches!(
        target.import(&COUNTER, &bytes, small, slot.clone(), None, 9),
        Err(BundleError::Container(_))
    ));
    // A bundle of another artifact is refused against the importing domain.
    assert!(matches!(
        target.import(&Counter::new(10), &bytes, limits, slot.clone(), None, 9),
        Err(BundleError::Store(HistoryError::ArtifactMismatch { .. }))
    ));
    assert!(target.is_empty_except_meta().unwrap());

    let (value, loaded) = target
        .import(&COUNTER, &bytes, limits, slot.clone(), None, 10)
        .unwrap();
    assert_eq!(
        (value.commit, loaded.state, loaded.header.depth),
        (commits[2], 3, 2)
    );
    assert_eq!(target.read_ref(&slot).unwrap(), Some(value));
    // The import recorded the transitions it carried.
    let (mut session, root) =
        Session::create(&mut target, COUNTER, name("player"), &0, 11).unwrap();
    assert_eq!(root.commit, commits[0]);
    let one = session
        .advance(&mut target, root.commit, &1, never, 12)
        .unwrap();
    assert_eq!(one.commit, commits[1]);

    // A receiver that holds the parent's closure gets only the new objects.
    let has = source
        .export(commits[2], &BTreeSet::new())
        .unwrap()
        .manifest
        .objects
        .iter()
        .map(|descriptor| descriptor.id)
        .collect();
    let increment = source.export(commits[3], &has).unwrap();
    assert_eq!(increment.objects.len(), 3);
    let increment = increment.to_bytes().unwrap();
    let (value, loaded) = target
        .import(
            &COUNTER,
            &increment,
            limits,
            slot.clone(),
            Some(value.revision),
            13,
        )
        .unwrap();
    assert_eq!((value.commit, loaded.state), (commits[3], 6));
    // A receiver without them is told what is missing.
    let empty_store = F::new();
    let mut empty = open(&empty_store);
    assert!(matches!(
        empty.import(&COUNTER, &increment, limits, slot, None, 14),
        Err(BundleError::MissingObject(_))
    ));
}

/// The same commit and input led somewhere else in the store a bundle came from.
fn imports_that_contradict_recorded_transitions_are_refused<F: Fixture>() {
    let here_store = F::new();
    let mut here = open(&here_store);
    let (_, recorded) = chain(&mut here, "player", &[1]);

    let there_store = F::new();
    let mut there = open(&there_store);
    let (mut session, root) = Session::create(&mut there, COUNTER, name("player"), &0, 1).unwrap();
    assert_eq!(root.commit, recorded[0]);
    let other = session
        .advance(
            &mut there,
            root.commit,
            &1,
            |_, state, input| Ok::<_, Overflow>(state + input + 6),
            2,
        )
        .unwrap();
    let bytes = there
        .export(other.commit, &BTreeSet::new())
        .unwrap()
        .to_bytes()
        .unwrap();

    let slot = RefKey::save(name("player"), name("slot"));
    let error = here
        .import(&COUNTER, &bytes, BundleLimits::default(), slot, None, 3)
        .unwrap_err();
    let BundleError::Store(HistoryError::Nondeterministic(found)) = error else {
        panic!("expected nondeterminism, got {error:?}");
    };
    assert_eq!(
        (found.parent, found.recorded, found.proposed),
        (recorded[0], recorded[1], other.commit)
    );
}

/// The engine checks structure; whether a state is well formed for its artifact is checked
/// whenever a session loads or imports it.
fn loading_checks_states_against_the_artifact<F: Fixture>() {
    let fixture = F::new();
    let mut history = open(&fixture);
    let (mut session, _) = Session::create(&mut history, COUNTER, name("player"), &0, 1).unwrap();
    // A root whose state passes the ceiling, written past the session API: 5000 in canonical
    // CBOR.
    let state = Object::new(COUNTER_STATE_KIND, 1, &[0x19, 0x13, 0x88]);
    let commit = Commit::root(COUNTER.artifact_id(), state.id()).to_object(COUNTER_COMMIT_KIND);
    let forged = commit.id();
    let slot = RefKey::save(name("player"), name("forged"));
    history
        .write(
            &Transaction {
                objects: vec![state.clone(), commit],
                refs: vec![RefMutation {
                    key: slot.clone(),
                    expected: None,
                    next: Some(forged),
                }],
                observed_at: 2,
                ..Transaction::default()
            },
            |_| Ok(Vec::new()),
        )
        .unwrap();
    assert!(matches!(
        history.load(&COUNTER, forged),
        Err(HistoryError::Corrupt(id, _)) if id == state.id()
    ));
    let head = session.head();
    assert!(matches!(
        session.checkout(&mut history, head, forged, 3),
        Err(HistoryError::Corrupt(..))
    ));
    let bytes = history
        .export(forged, &BTreeSet::new())
        .unwrap()
        .to_bytes()
        .unwrap();
    let other_store = F::new();
    let mut other = open(&other_store);
    assert!(matches!(
        other.import(&COUNTER, &bytes, BundleLimits::default(), slot, None, 4),
        Err(BundleError::Store(HistoryError::Corrupt(..)))
    ));
    assert!(other.is_empty_except_meta().unwrap());
}

fn verify_path_replays_on_request<F: Fixture>() {
    let fixture = F::new();
    let mut history = open(&fixture);
    let (_, commits) = chain(&mut history, "player", &[1, 2, 3, 4]);
    history.verify_path(&COUNTER, commits[4], step).unwrap();
    let doubled = |_: &Counter, state: &u64, input: &u64| Ok::<_, Overflow>(state + 2 * input);
    assert!(matches!(
        history.verify_path(&COUNTER, commits[4], doubled),
        Err(AuditError::Diverged { commit, .. }) if commit == commits[1]
    ));
    let refuse = |_: &Counter, _: &u64, _: &u64| Err::<u64, _>(Overflow);
    assert_eq!(
        history.verify_path(&COUNTER, commits[2], refuse),
        Err(AuditError::Step(Overflow))
    );
    history.verify_path(&COUNTER, commits[0], refuse).unwrap();
}
