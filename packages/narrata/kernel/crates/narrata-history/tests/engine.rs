//! The engine under a registered domain: write checks, GC, the integrity scan and unknown
//! outcomes, on every backend (ADR 0014, ADR 0015).

#![allow(clippy::panic, clippy::unwrap_used)]

#[macro_use]
mod support;

use std::convert::Infallible;

use narrata_history::{
    Commit, Domain, DomainKinds, History, HistoryError, KindInfo, Object, ObjectId, Op, Pin,
    Reader, RefKey, RefMutation, Reference, Registry, RetentionPolicy, Session, Transaction, View,
    domain_kinds,
    testing::{COUNTER_COMMIT_KIND, COUNTER_INPUT_KIND, COUNTER_STATE_KIND, Counter},
};
use narrata_storage::{
    Conflict, StorageBackend, StorageError,
    testing::{Fault, FaultInjecting, FaultPlan, Primitive, Trigger},
};
use support::{COUNTER, Counters, Fixture, chain, heads, name, never, open, step};

on_both_backends!(
    writes_check_the_commit_graph,
    gc_keeps_every_branch_head,
    gc_refuses_kinds_no_registrant_knows,
    unknown_outcomes_are_reconciled,
);

const NOTE_KIND: u16 = 0xFF20;

/// The counter domain plus a leaf kind the counter registry does not know.
struct WithNotes;

impl Registry for WithNotes {
    type Error = HistoryError;
    type Tag = Infallible;

    fn kind(&self, code: u16) -> Option<KindInfo> {
        (code == NOTE_KIND)
            .then_some(KindInfo {
                name: "note",
                leaf: true,
                commit: false,
            })
            .or_else(|| domain_kinds::kind::<Counter>(code))
    }

    fn references(&self, object: &Object) -> Result<Vec<Reference>, HistoryError> {
        domain_kinds::references::<Counter>(object)
    }

    fn validate<B: StorageBackend>(
        &self,
        object: &Object,
        view: &mut View<'_, B, Self>,
    ) -> Result<Vec<Op<Infallible>>, HistoryError> {
        if object.kind() == NOTE_KIND {
            return Ok(Vec::new());
        }
        domain_kinds::validate::<Counter, B, Self>(object, view)
    }

    fn unindex<B: StorageBackend>(
        &self,
        object: &Object,
        reader: &Reader<'_, B>,
    ) -> Result<Vec<Op<Infallible>>, HistoryError> {
        domain_kinds::unindex::<Counter, B, Infallible>(object, reader)
    }

    fn conflict(&self, tag: &Infallible, _: Conflict) -> HistoryError {
        match *tag {}
    }
}

fn state(value: u8) -> Object {
    Object::new(COUNTER_STATE_KIND, 1, &[value])
}

fn input(value: u8) -> Object {
    Object::new(COUNTER_INPUT_KIND, 1, &[value])
}

fn write<B: StorageBackend>(
    history: &mut Counters<B>,
    objects: Vec<Object>,
) -> Result<(), HistoryError> {
    history
        .write(
            &Transaction {
                objects,
                observed_at: 1,
                ..Transaction::default()
            },
            |_| Ok(Vec::new()),
        )
        .map(drop)
}

fn writes_check_the_commit_graph<F: Fixture>() {
    let fixture = F::new();
    let mut history = open(&fixture);
    let (_, commits) = chain(&mut history, "player", &[1]);
    let artifact = COUNTER.artifact_id();
    let child = |parent: ObjectId, depth: u64, artifact, state: &Object, input: &Object| {
        Commit {
            artifact,
            parent: Some(parent),
            input: Some(input.id()),
            state: state.id(),
            depth,
        }
        .to_object(COUNTER_COMMIT_KIND)
    };
    let (two, increment) = (state(2), input(1));

    // The parent must exist.
    let orphan = child(ObjectId::from_bytes([9; 32]), 1, artifact, &two, &increment);
    assert_eq!(
        write(&mut history, vec![two.clone(), increment.clone(), orphan]),
        Err(HistoryError::MissingObject(ObjectId::from_bytes([9; 32])))
    );
    // It must name the same artifact, one step shallower.
    let other = child(
        commits[1],
        2,
        Counter::new(10).artifact_id(),
        &two,
        &increment,
    );
    assert!(matches!(
        write(&mut history, vec![two.clone(), increment.clone(), other]),
        Err(HistoryError::InvalidGraph(_))
    ));
    let deep = child(commits[1], 5, artifact, &two, &increment);
    assert!(matches!(
        write(&mut history, vec![two.clone(), increment.clone(), deep]),
        Err(HistoryError::InvalidGraph(_))
    ));
    // The state and input fields name objects of those kinds.
    let swapped = child(commits[1], 2, artifact, &increment, &two);
    assert!(matches!(
        write(&mut history, vec![two.clone(), increment.clone(), swapped]),
        Err(HistoryError::ObjectKind(_))
    ));
    // A root has neither parent nor input; anything else does not decode.
    let mut payload = Commit::root(artifact, two.id()).encode();
    *payload.last_mut().unwrap() = 1; // depth 1
    let malformed = Object::new(COUNTER_COMMIT_KIND, 1, &payload);
    assert!(matches!(
        write(&mut history, vec![two.clone(), malformed]),
        Err(HistoryError::Corrupt(..))
    ));
    // Unregistered kinds are refused, and Refs name commits only.
    let note = Object::new(NOTE_KIND, 1, &[0xa0]);
    assert!(matches!(
        write(&mut history, vec![note]),
        Err(HistoryError::UnregisteredKind {
            kind: NOTE_KIND,
            ..
        })
    ));
    write(&mut history, vec![two.clone()]).unwrap();
    assert_eq!(
        history.write(
            &Transaction {
                refs: vec![RefMutation {
                    key: RefKey::save(name("player"), name("slot")),
                    expected: None,
                    next: Some(two.id()),
                }],
                ..Transaction::default()
            },
            |_| Ok(Vec::new()),
        ),
        Err(HistoryError::ObjectKind(two.id()))
    );
    // A well-formed child of the head is accepted and indexed.
    let good = child(commits[1], 2, artifact, &two, &increment);
    write(&mut history, vec![increment.clone(), good.clone()]).unwrap();
    assert_eq!(
        history.transition(commits[1], increment.id()).unwrap(),
        Some(good.id())
    );
    assert!(history.integrity_scan().unwrap().is_empty());
}

fn gc_keeps_every_branch_head<F: Fixture>() {
    let fixture = F::new();
    let mut history = open(&fixture);
    let (mut player, commits) = chain(&mut history, "player", &[1, 2]);
    player
        .checkout(&mut history, commits[2], commits[0], 4)
        .unwrap();
    let fork = player
        .advance(&mut history, commits[0], &5, step, 5)
        .unwrap();
    let kept = heads(&history, "player");
    assert_eq!(kept.len(), 2);

    // A session abandoned by deleting its Refs.
    let (mut scratch, _) = Session::create(&mut history, COUNTER, name("scratch"), &7, 6).unwrap();
    let scratch_root = scratch.head();
    let scratch_child = scratch
        .advance(&mut history, scratch_root, &1, step, 7)
        .unwrap();
    let scratch_input = Object::new(COUNTER_INPUT_KIND, 1, &[0x01]).id();
    assert_eq!(
        history.transition(scratch_root, scratch_input).unwrap(),
        Some(scratch_child.commit)
    );
    let mut refs = vec![RefKey::active(name("scratch")).unwrap()];
    refs.extend(
        heads(&history, "scratch")
            .into_keys()
            .map(|branch| RefKey::branch(name("scratch"), branch)),
    );
    let refs = refs
        .into_iter()
        .map(|key| RefMutation {
            expected: Some(history.read_ref(&key).unwrap().unwrap().revision),
            key,
            next: None,
        })
        .collect();
    history
        .write(
            &Transaction {
                refs,
                ..Transaction::default()
            },
            |_| Ok(Vec::new()),
        )
        .unwrap();

    // Inside the grace period nothing goes.
    let young = history
        .collect(RetentionPolicy {
            now: 8,
            grace_seconds: 10,
            dry_run: false,
        })
        .unwrap();
    assert!(young.removed.is_empty());
    let dry = history
        .collect(RetentionPolicy {
            dry_run: true,
            ..RetentionPolicy::default()
        })
        .unwrap();
    let report = history.collect(RetentionPolicy::default()).unwrap();
    assert_eq!(report.removed, dry.removed);
    // The scratch commits and their states go; the shared increment stays.
    let removed = report
        .removed
        .iter()
        .map(|(kind, removed)| (*kind, removed.objects))
        .collect::<Vec<_>>();
    assert_eq!(removed, [(COUNTER_COMMIT_KIND, 2), (COUNTER_STATE_KIND, 2)]);
    assert_eq!(history.get_object(scratch_root).unwrap(), None);
    assert_eq!(
        history.transition(scratch_root, scratch_input).unwrap(),
        None
    );
    assert!(
        history
            .children(scratch_root, None, 8)
            .unwrap()
            .items
            .is_empty()
    );

    // Every branch head and its history survive and still load.
    assert_eq!(heads(&history, "player"), kept);
    for commit in commits.iter().chain([&fork.commit]) {
        history.load(&COUNTER, *commit).unwrap();
    }
    assert!(history.integrity_scan().unwrap().is_empty());
    // With the transition gone, the same advance runs its step again.
    let (mut scratch, _) = Session::create(&mut history, COUNTER, name("scratch"), &7, 9).unwrap();
    let again = scratch
        .advance(&mut history, scratch_root, &1, step, 10)
        .unwrap();
    assert_eq!((again.commit, again.reused), (scratch_child.commit, false));
    scratch
        .checkout(&mut history, again.commit, scratch_root, 11)
        .unwrap();
    let reused = scratch
        .advance(&mut history, scratch_root, &1, never, 12)
        .unwrap();
    assert!(reused.reused);
}

fn gc_refuses_kinds_no_registrant_knows<F: Fixture>() {
    let fixture = F::new();
    let mut wide = History::open(fixture.backend(), WithNotes).unwrap();
    let note = Object::new(NOTE_KIND, 1, &[0xa0]);
    wide.write(
        &Transaction {
            objects: vec![note.clone()],
            pins: vec![Pin {
                owner: "notes".to_owned(),
                object: note.id(),
                expires_at: None,
            }],
            observed_at: 1,
            ..Transaction::default()
        },
        |_| Ok(Vec::new()),
    )
    .unwrap();
    assert!(
        wide.collect(RetentionPolicy::default())
            .unwrap()
            .removed
            .is_empty()
    );

    let mut narrow = open(&fixture);
    chain(&mut narrow, "player", &[1]);
    assert_eq!(
        narrow.collect(RetentionPolicy::default()),
        Err(HistoryError::UnregisteredKind {
            object: note.id(),
            kind: NOTE_KIND,
        })
    );
    let issues = narrow.integrity_scan().unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].object, note.id());
    // The refusal deleted nothing.
    assert!(wide.get_object(note.id()).unwrap().is_some());
}

fn unknown_outcomes_are_reconciled<F: Fixture>() {
    let fixture = F::new();
    let mut history = History::open(
        FaultInjecting::new(fixture.backend(), FaultPlan::new()),
        DomainKinds::<Counter>::new(),
    )
    .unwrap();
    // The first batch applies but reports an I/O error; reading back shows it applied.
    history
        .backend_mut()
        .set_plan(FaultPlan::new().at(Trigger::Nth(Primitive::Apply, 1), Fault::After));
    let (mut session, root) =
        Session::create(&mut history, COUNTER, name("player"), &0, 1).unwrap();
    history.backend_mut().set_plan(FaultPlan::new());
    assert_eq!(history.load(&COUNTER, root.commit).unwrap().state, 0);

    // A batch that fails before applying is reported, and nothing moved.
    history.backend_mut().set_plan(FaultPlan::new().at(
        Trigger::Nth(Primitive::Apply, 1),
        Fault::Before(StorageError::Io("disk".to_owned())),
    ));
    assert!(
        session
            .advance(&mut history, root.commit, &1, step, 2)
            .is_err()
    );
    history.backend_mut().set_plan(FaultPlan::new());
    let (session, _) = Session::open(&history, COUNTER, name("player")).unwrap();
    assert_eq!(session.head(), root.commit);
    assert!(
        history
            .children(root.commit, None, 8)
            .unwrap()
            .items
            .is_empty()
    );
}
