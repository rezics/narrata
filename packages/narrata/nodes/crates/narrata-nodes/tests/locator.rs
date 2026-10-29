#![allow(clippy::unwrap_used)]
mod support;

use narrata_history::{RefKey, RefName, layout};
use narrata_nodes::{
    Session,
    history::locator::{self, Limits},
};
use narrata_storage::{Batch, Expect, MemoryBackend, ObjectDigest, StorageBackend};

#[test]
fn discovery_is_read_only_and_distinguishes_missing_corrupt_and_new() {
    let mut backend = MemoryBackend::new();
    let execution = support::execution();
    assert_eq!(locator::locate(&backend, execution).unwrap(), None);
    assert!(
        backend
            .read_key(layout::META, layout::LAYOUT_KEY)
            .unwrap()
            .is_none()
    );
    let (program, _) = support::open(&support::compiled().pack);
    let session = Session::open(program.clone(), execution, backend.share()).unwrap();
    let located = locator::locate(&backend, execution).unwrap().unwrap();
    assert_eq!(located.cursor, session.cursor().unwrap());
    assert_eq!(located.artifact, program.artifact_id());
    backend
        .apply(&Batch::new().delete_object(ObjectDigest::from_bytes(*located.cursor.as_bytes())))
        .unwrap();
    assert!(locator::locate(&backend, execution).is_err());
    let key = layout::ref_key(&RefKey::active(RefName::hex(execution.as_bytes())).unwrap());
    backend
        .apply(&Batch::new().put(layout::REFS, key, vec![0xff], Expect::Any))
        .unwrap();
    assert!(locator::locate(&backend, execution).is_err());
}

#[test]
fn proof_requires_same_execution_roots_and_follows_descendants_with_a_budget() {
    let mut backend = MemoryBackend::new();
    let (program, names) = support::open(&support::compiled().pack);
    let execution = support::execution();
    let mut session = Session::open(program, execution, backend.share()).unwrap();
    support::choose(&mut session, &names, &["wave"]).unwrap();
    let candidate = session.cursor().unwrap();
    support::choose(&mut session, &names, &[]).unwrap();
    assert!(locator::retained(&backend, execution, candidate, Limits::default()).unwrap());
    assert!(
        locator::retained(
            &backend,
            execution,
            candidate,
            Limits {
                roots: 0,
                commits: 0
            }
        )
        .is_err()
    );
    let other = "execution:0190f2a0000070008000000000000002"
        .parse()
        .unwrap();
    assert!(!locator::retained(&backend, other, candidate, Limits::default()).unwrap());
    // Keep every object, remove only this execution's roots: object existence proves nothing.
    let entries = backend
        .scan_keys(layout::REFS, &[], None, 128)
        .unwrap()
        .entries;
    for entry in entries {
        backend
            .apply(&Batch::new().delete(layout::REFS, entry.key, Expect::Any))
            .unwrap();
    }
    assert!(!locator::retained(&backend, execution, candidate, Limits::default()).unwrap());
}
