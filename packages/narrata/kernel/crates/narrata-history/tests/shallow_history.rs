#![allow(clippy::panic, clippy::unwrap_used)]

#[macro_use]
mod support;

use std::collections::BTreeSet;

use narrata_history::testing::Counter;
use narrata_history::{
    AuditError, BundleError, BundleLimits, Commit, Domain, HistoryError, Object, RefKey,
    RefMutation, SHALLOW_MAGIC, Session, ShallowBundle, TRUNCATED_PARENT_KIND, Transaction,
};
use narrata_kernel::codec::CborWriter;
use narrata_storage::{Batch, StorageBackend};
use support::{COUNTER, Fixture, chain, name, never, open, step};

on_both_backends!(
    shallow_restore_and_hydration,
    bounds_and_checked_import,
    gc_removes_unrooted_shallow_history
);

fn shallow_restore_and_hydration<F: Fixture>() {
    let source = F::new();
    let mut source = open(&source);
    let (_, commits) = chain(&mut source, "source", &[1, 2, 3, 4]);
    let head = commits[4];
    let shallow = source
        .export_shallow(head, 1, &BTreeSet::new())
        .unwrap()
        .to_bytes()
        .unwrap();
    assert!(shallow.starts_with(SHALLOW_MAGIC));
    assert_eq!(
        shallow,
        source
            .export_shallow(head, 1, &BTreeSet::new())
            .unwrap()
            .to_bytes()
            .unwrap()
    );
    let full = source
        .export(head, &BTreeSet::new())
        .unwrap()
        .to_bytes()
        .unwrap();
    let destination = F::new();
    let mut history = open(&destination);
    let slot = RefKey::save(name("restored"), name("slot"));
    let (value, loaded) = history
        .import(
            &COUNTER,
            &shallow,
            BundleLimits::default(),
            slot.clone(),
            None,
            10,
        )
        .unwrap();
    assert_eq!(
        (loaded.commit, loaded.state, loaded.header.depth),
        (head, 10, 4)
    );
    assert_eq!(history.load(&COUNTER, commits[3]).unwrap().state, 6);
    assert_eq!(
        history.load(&COUNTER, commits[2]),
        Err(HistoryError::HistoryTruncated(commits[2]))
    );
    assert!(
        matches!(history.verify_path(&COUNTER, head, step), Err(AuditError::History(HistoryError::HistoryTruncated(id))) if id == commits[2])
    );
    assert!(
        matches!(history.export(head, &BTreeSet::new()), Err(BundleError::Store(HistoryError::HistoryTruncated(id))) if id == commits[2])
    );
    assert!(history.integrity_scan().unwrap().is_empty());
    history.collect(Default::default()).unwrap();
    drop(history);
    let mut history = open(&destination);
    assert_eq!(
        history.load(&COUNTER, commits[2]),
        Err(HistoryError::HistoryTruncated(commits[2]))
    );
    // A shallow store can be re-exported even when a larger ancestor budget meets its boundary.
    assert_eq!(
        history
            .export_shallow(head, 100, &BTreeSet::new())
            .unwrap()
            .to_bytes()
            .unwrap(),
        shallow
    );
    assert_eq!(
        history.children(commits[2], None, 1).unwrap().items,
        vec![commits[3]]
    );
    assert!(
        history
            .children(commits[2], Some(&commits[3]), 1)
            .unwrap()
            .items
            .is_empty()
    );
    let (mut player, _) = Session::create(&mut history, COUNTER, name("player"), &0, 11).unwrap();
    let original = player.head();
    player.checkout(&mut history, original, head, 12).unwrap();
    player.checkout(&mut history, head, commits[3], 13).unwrap();
    assert!(
        matches!(player.checkout(&mut history, commits[3], commits[2], 14), Err(HistoryError::HistoryTruncated(id)) if id == commits[2])
    );
    assert_eq!(player.head(), commits[3]);
    let reused = player
        .advance(&mut history, commits[3], &4, never, 15)
        .unwrap();
    assert_eq!((reused.commit, reused.reused), (head, true));
    let next = player.advance(&mut history, head, &5, step, 16).unwrap();
    assert_eq!((next.state, next.depth), (15, 5));
    player
        .checkout(&mut history, next.commit, commits[3], 17)
        .unwrap();
    let fork = player
        .advance(&mut history, commits[3], &9, step, 18)
        .unwrap();
    let page = history.children(commits[3], None, 1).unwrap();
    assert!(page.more);
    let second = history
        .children(commits[3], Some(&page.items[0]), 1)
        .unwrap();
    assert!(!second.more);
    assert_eq!(
        page.items
            .into_iter()
            .chain(second.items)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([head, fork.commit])
    );
    history.collect(Default::default()).unwrap();
    assert!(history.integrity_scan().unwrap().is_empty());
    history
        .import(
            &COUNTER,
            &full,
            BundleLimits::default(),
            slot,
            Some(value.revision),
            17,
        )
        .unwrap();
    history.verify_path(&COUNTER, next.commit, step).unwrap();
    history.verify_path(&COUNTER, fork.commit, step).unwrap();
    assert_eq!(history.load(&COUNTER, commits[2]).unwrap().state, 3);
    assert_eq!(
        history
            .export(head, &BTreeSet::new())
            .unwrap()
            .to_bytes()
            .unwrap(),
        full
    );
    let report = history.collect(Default::default()).unwrap();
    assert_eq!(report.removed[&TRUNCATED_PARENT_KIND].objects, 1);
    assert!(history.integrity_scan().unwrap().is_empty());
}

fn bounds_and_checked_import<F: Fixture>() {
    let source = F::new();
    let mut source = open(&source);
    let (_, commits) = chain(&mut source, "source", &[1, 2, 3]);
    for (budget, count, boundary) in [
        (0, 1, vec![commits[2]]),
        (1, 2, vec![commits[1]]),
        (3, 4, vec![]),
        (u64::MAX, 4, vec![]),
    ] {
        let bundle = source
            .export_shallow(commits[3], budget, &BTreeSet::new())
            .unwrap();
        assert_eq!(bundle.manifest.boundary_parents, boundary);
        assert_eq!(
            bundle
                .objects
                .iter()
                .filter(|object| object.kind() == <Counter as Domain>::COMMIT_KIND)
                .count(),
            count
        );
    }
    let root = source
        .export_shallow(commits[0], 0, &BTreeSet::new())
        .unwrap();
    assert!(root.manifest.boundary_parents.is_empty());
    let valid = source
        .export_shallow(commits[3], 0, &BTreeSet::new())
        .unwrap();
    let destination = F::new();
    let mut history = open(&destination);
    let slot = || RefKey::save(name("import"), name("slot"));
    let refuse = |history: &mut support::Counters<F::Backend>, bytes: &[u8]| {
        assert!(
            history
                .import(&COUNTER, bytes, BundleLimits::default(), slot(), None, 10)
                .is_err()
        );
        assert!(history.is_empty_except_meta().unwrap());
    };
    let mut tampered = valid.to_bytes().unwrap();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    refuse(&mut history, &tampered);
    let mut no_boundary = valid.clone();
    no_boundary.manifest.boundary_parents.clear();
    refuse(&mut history, &no_boundary.to_bytes().unwrap());
    let mut unused = valid.clone();
    unused.manifest.boundary_parents.push(commits[0]);
    unused.manifest.boundary_parents.sort();
    refuse(&mut history, &unused.to_bytes().unwrap());
    let mut missing_state = valid.clone();
    let header = Commit::from_object(
        &source.get_object(commits[3]).unwrap().unwrap(),
        <Counter as Domain>::COMMIT_KIND,
    )
    .unwrap();
    missing_state
        .manifest
        .checkpoint
        .objects
        .retain(|descriptor| descriptor.id != header.state);
    missing_state
        .objects
        .retain(|object| object.id() != header.state);
    missing_state.manifest.boundary_parents.push(header.state);
    missing_state.manifest.boundary_parents.sort();
    refuse(&mut history, &missing_state.to_bytes().unwrap());
    let mut duplicate = valid.manifest.clone();
    duplicate
        .boundary_parents
        .push(duplicate.boundary_parents[0]);
    assert!(duplicate.to_object().is_err());
    assert!(narrata_history::ShallowManifest::decode(&duplicate.encode()).is_err());
    let mut overlap = valid.manifest.clone();
    overlap.boundary_parents = vec![commits[3]];
    assert!(overlap.to_object().is_err());
    let bytes = valid.to_bytes().unwrap();
    assert!(
        ShallowBundle::from_bytes(
            &bytes,
            BundleLimits {
                max_total_bytes: 1,
                ..Default::default()
            },
            |_| true
        )
        .is_err()
    );
    refuse(&mut history, &bytes[..bytes.len() - 1]);
    // Delta shallow bundles keep descriptors, and need the advertised receiver objects.
    let receiver_has = valid.objects.iter().map(Object::id).collect();
    let delta = source
        .export_shallow(commits[3], 0, &receiver_has)
        .unwrap()
        .to_bytes()
        .unwrap();
    refuse(&mut history, &delta);
    let (value, _) = history
        .import(&COUNTER, &bytes, BundleLimits::default(), slot(), None, 10)
        .unwrap();
    history
        .import(
            &COUNTER,
            &delta,
            BundleLimits::default(),
            slot(),
            Some(value.revision),
            11,
        )
        .unwrap();
    // Deleting a carried state remains corruption, even though the store has a valid boundary.
    history
        .backend_mut()
        .apply(
            &Batch::new().delete_object(narrata_storage::ObjectDigest::from_bytes(
                *header.state.as_bytes(),
            )),
        )
        .unwrap();
    assert!(
        matches!(history.load(&COUNTER, commits[3]), Err(HistoryError::MissingObject(id)) if id == header.state)
    );
    assert!(!history.integrity_scan().unwrap().is_empty());
}

fn gc_removes_unrooted_shallow_history<F: Fixture>() {
    let source = F::new();
    let mut source = open(&source);
    let (_, commits) = chain(&mut source, "source", &[1, 2]);
    let bundle = source
        .export_shallow(commits[2], 0, &BTreeSet::new())
        .unwrap()
        .to_bytes()
        .unwrap();
    let destination = F::new();
    let mut history = open(&destination);
    let slot = RefKey::save(name("import"), name("slot"));
    let (value, _) = history
        .import(
            &COUNTER,
            &bundle,
            BundleLimits::default(),
            slot.clone(),
            None,
            10,
        )
        .unwrap();
    history
        .write(
            &Transaction {
                refs: vec![RefMutation {
                    key: slot,
                    expected: Some(value.revision),
                    next: None,
                }],
                ..Default::default()
            },
            |_| Ok(vec![]),
        )
        .unwrap();
    let report = history.collect(Default::default()).unwrap();
    assert_eq!(report.removed[&TRUNCATED_PARENT_KIND].objects, 1);
    assert!(
        history
            .backend()
            .scan_objects(None, 100)
            .unwrap()
            .digests
            .is_empty()
    );
    assert!(
        history
            .children(commits[1], None, 10)
            .unwrap()
            .items
            .is_empty()
    );
    assert!(history.integrity_scan().unwrap().is_empty());
}

#[test]
fn shallow_import_orders_truncation_evidence_before_commits_in_split_batches() {
    let mut source = narrata_history::History::in_memory(narrata_history::DomainKinds::new());
    let (_, commits) = chain(&mut source, "source", &[1, 2, 3]);
    let bytes = source
        .export_shallow(commits[3], 1, &BTreeSet::new())
        .unwrap()
        .to_bytes()
        .unwrap();
    let mut history = narrata_history::History::open(
        narrata_storage::MemoryBackend::with_limits(narrata_storage::Limits {
            max_batch_ops: 8,
            ..Default::default()
        }),
        narrata_history::DomainKinds::<Counter>::new(),
    )
    .unwrap();
    history
        .import(
            &COUNTER,
            &bytes,
            BundleLimits::default(),
            RefKey::save(name("import"), name("slot")),
            None,
            1,
        )
        .unwrap();
    assert_eq!(history.load(&COUNTER, commits[3]).unwrap().state, 6);
    history.collect(Default::default()).unwrap();
    assert!(history.integrity_scan().unwrap().is_empty());
}

#[test]
fn truncation_objects_are_checked_before_writing() {
    let mut writer = CborWriter::new();
    writer.map(1);
    writer.unsigned(0);
    writer.bytes(&[0; 31]);
    let mut history =
        narrata_history::History::in_memory(narrata_history::DomainKinds::<Counter>::new());
    assert!(
        history
            .write(
                &Transaction {
                    objects: vec![Object::new(TRUNCATED_PARENT_KIND, 1, &writer.into_bytes())],
                    ..Default::default()
                },
                |_| Ok(vec![])
            )
            .is_err()
    );
    assert!(history.is_empty_except_meta().unwrap());
}
