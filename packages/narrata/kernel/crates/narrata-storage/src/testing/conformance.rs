//! Contract cases any atomic backend must pass.
//!
//! Each case is a public function over a [`Harness`]; [`conformance_tests!`](crate::conformance_tests)
//! expands all of them into one `#[test]` each. Cases that need a capability the backend does
//! not declare (storage that outlives its handles, concurrent writers) pass without checking
//! anything.

// The suite reports failures by panicking, like any test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::{
    sync::{Arc, Barrier},
    thread,
};

pub use super::model::random_batches_match_model;
use crate::{
    Applied, Batch, Conflict, Durability, Expect, KeyAction, KeyEntry, KeyOp, KeySpace, KeyValue,
    Limit, LimitExceeded, ObjectDigest, Revision, StorageBackend, StorageError,
};

/// Creates and reopens the stores a backend's cases run against.
pub trait Harness {
    type Backend: StorageBackend + Send;

    /// Creates a new, empty store.
    fn create(&mut self) -> Self::Backend;

    /// Opens another live handle to the store behind `backend`. Required when the backend
    /// declares concurrent writers.
    fn connect(&mut self, _backend: &Self::Backend) -> Option<Self::Backend> {
        None
    }

    /// Closes `backend` and opens its store again. Required when the backend's data outlives its
    /// handles, that is when it declares durable or evictable storage.
    fn reopen(&mut self, _backend: Self::Backend) -> Option<Self::Backend> {
        None
    }
}

/// Expands into one `#[test]` per conformance case, each with a fresh harness from `$harness`.
#[macro_export]
macro_rules! conformance_tests {
    ($harness:expr) => {
        $crate::conformance_tests!(@cases $harness;
            object_put_is_idempotent,
            object_delete_removes_only_named_objects,
            object_reads_align_with_request,
            object_scan_pages_in_digest_order,
            key_put_checks_each_precondition,
            key_delete_and_check_follow_preconditions,
            revisions_increase_and_are_never_reused,
            failing_operation_leaves_batch_without_effect,
            malformed_requests_are_rejected_without_effect,
            key_scan_orders_filters_and_resumes,
            limits_reject_requests_without_effect,
            reopened_store_keeps_state_and_revisions,
            concurrent_handles_race_for_one_cas,
            random_batches_match_model,
        );
    };
    (@cases $harness:expr; $($case:ident),* $(,)?) => {
        $(
            #[test]
            fn $case() {
                $crate::testing::conformance::$case(&mut $harness);
            }
        )*
    };
}

pub(super) const A: KeySpace = KeySpace::new(0);
pub(super) const B: KeySpace = KeySpace::new(1);
pub(super) const LAST: KeySpace = KeySpace::new(u16::MAX);

/// A revision no backend issues within a test.
pub(super) fn never_issued() -> Revision {
    Revision::new(u64::MAX).expect("u64::MAX is non-zero")
}

/// Digest whose order differs from `n`'s, so that scans cannot pass by insertion order.
pub(super) fn digest(n: u64) -> ObjectDigest {
    let mut state = n;
    let mut bytes = [0; 32];
    for chunk in bytes.chunks_mut(8) {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = state;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        chunk.copy_from_slice(&(mixed ^ (mixed >> 31)).to_be_bytes());
    }
    ObjectDigest::from_bytes(bytes)
}

fn payload(n: u64) -> Arc<[u8]> {
    format!("object {n}").into_bytes().into()
}

fn create<H: Harness>(harness: &mut H) -> H::Backend {
    let backend = harness.create();
    assert!(
        backend.capabilities().atomic_multi_key(),
        "the suite covers atomic backends; RootKeyOnly backends need their own cases (ADR 0012)"
    );
    backend
}

fn revision(applied: Applied) -> Revision {
    applied
        .revision
        .expect("a batch that puts keys returns their revision")
}

fn conflict<T: std::fmt::Debug>(result: Result<T, StorageError>) -> Conflict {
    match result {
        Err(StorageError::Conflict(conflict)) => *conflict,
        other => panic!("expected a precondition conflict, got {other:?}"),
    }
}

fn limit<T: std::fmt::Debug>(result: Result<T, StorageError>, expected: Limit) {
    match result {
        Err(StorageError::Limit(LimitExceeded { limit, .. })) if limit == expected => {}
        other => panic!("expected the {expected:?} limit, got {other:?}"),
    }
}

fn invalid<T: std::fmt::Debug>(result: Result<T, StorageError>) {
    assert!(
        matches!(result, Err(StorageError::Invalid(_))),
        "expected an invalid request, got {result:?}"
    );
}

fn value(backend: &impl StorageBackend, space: KeySpace, key: &[u8]) -> Option<KeyValue> {
    backend.read_key(space, key).unwrap()
}

fn object(backend: &impl StorageBackend, digest: ObjectDigest) -> Option<Vec<u8>> {
    backend
        .get_object(&digest)
        .unwrap()
        .map(|bytes| bytes.to_vec())
}

/// Pages through a key scan, checking page bounds and the exact continuation flag.
pub(super) fn collect_keys(
    backend: &impl StorageBackend,
    space: KeySpace,
    prefix: &[u8],
    after: Option<&[u8]>,
    limit: u32,
) -> Vec<KeyEntry> {
    let mut entries: Vec<KeyEntry> = Vec::new();
    let mut cursor = after.map(<[u8]>::to_vec);
    for page_number in 0.. {
        let page = backend
            .scan_keys(space, prefix, cursor.as_deref(), limit)
            .unwrap();
        assert!(
            page.entries.len() <= limit as usize,
            "page exceeds its limit"
        );
        assert!(
            !page.more || page.entries.len() == limit as usize,
            "a page with more entries pending must be full"
        );
        assert!(
            page_number == 0 || !page.entries.is_empty(),
            "the previous page announced more entries that did not come"
        );
        for entry in &page.entries {
            assert!(entry.key.starts_with(prefix), "entry outside the prefix");
            if let Some(previous) = entries.last() {
                assert!(previous.key < entry.key, "keys out of order");
            }
        }
        entries.extend(page.entries.iter().cloned());
        match page.resume_after() {
            Some(last) => cursor = Some(last.to_vec()),
            None => break,
        }
    }
    entries
}

/// Pages through the object scan, checking page bounds and the exact continuation flag.
pub(super) fn collect_digests(backend: &impl StorageBackend, limit: u32) -> Vec<ObjectDigest> {
    let mut digests: Vec<ObjectDigest> = Vec::new();
    let mut cursor = None;
    for page_number in 0.. {
        let page = backend.scan_objects(cursor.as_ref(), limit).unwrap();
        assert!(
            page.digests.len() <= limit as usize,
            "page exceeds its limit"
        );
        assert!(
            !page.more || page.digests.len() == limit as usize,
            "a page with more digests pending must be full"
        );
        assert!(
            page_number == 0 || !page.digests.is_empty(),
            "the previous page announced more digests that did not come"
        );
        for digest in &page.digests {
            if let Some(previous) = digests.last() {
                assert!(previous < digest, "digests out of order");
            }
        }
        digests.extend(page.digests.iter().copied());
        match page.resume_after() {
            Some(last) => cursor = Some(*last),
            None => break,
        }
    }
    digests
}

/// Everything the suite can observe in the key spaces it uses.
#[derive(Debug, Eq, PartialEq)]
struct Dump {
    objects: Vec<(ObjectDigest, Vec<u8>)>,
    keys: Vec<(KeySpace, Vec<KeyEntry>)>,
}

fn dump(backend: &impl StorageBackend) -> Dump {
    let page = u32::try_from(backend.capabilities().limits.max_read_items)
        .unwrap_or(u32::MAX)
        .min(64);
    let digests = collect_digests(backend, page);
    let objects = digests
        .chunks(page as usize)
        .flat_map(|chunk| {
            let bytes = backend.get_objects(chunk).unwrap();
            chunk.iter().copied().zip(bytes).collect::<Vec<_>>()
        })
        .map(|(digest, bytes)| (digest, bytes.expect("scanned object exists").to_vec()))
        .collect();
    let keys = [A, B, LAST]
        .into_iter()
        .map(|space| (space, collect_keys(backend, space, b"", None, page)))
        .collect();
    Dump { objects, keys }
}

pub fn object_put_is_idempotent<H: Harness>(harness: &mut H) {
    let mut backend = create(harness);
    let one = digest(1);
    let first = backend
        .apply(
            &Batch::new()
                .put_object(one, payload(1))
                .put_object(digest(2), Vec::new()),
        )
        .unwrap();
    assert_eq!(
        first,
        Applied {
            revision: None,
            objects_inserted: 2,
            objects_deleted: 0,
        }
    );
    let again = backend
        .apply(&Batch::new().put_object(one, payload(1)))
        .unwrap();
    assert_eq!(again.objects_inserted, 0);

    // Existing digests are skipped without comparing bytes; the engine verifies on read.
    let other = backend
        .apply(&Batch::new().put_object(one, payload(9)))
        .unwrap();
    assert_eq!(other.objects_inserted, 0);
    assert_eq!(object(&backend, one), Some(payload(1).to_vec()));
    assert_eq!(object(&backend, digest(2)), Some(Vec::new()));
}

pub fn object_delete_removes_only_named_objects<H: Harness>(harness: &mut H) {
    let mut backend = create(harness);
    let batch = (1..=3).fold(Batch::new(), |batch, n| {
        batch.put_object(digest(n), payload(n))
    });
    backend.apply(&batch).unwrap();

    let applied = backend
        .apply(
            &Batch::new()
                .delete_object(digest(2))
                .delete_object(digest(9)),
        )
        .unwrap();
    assert_eq!(
        applied,
        Applied {
            revision: None,
            objects_inserted: 0,
            objects_deleted: 1,
        }
    );
    assert_eq!(object(&backend, digest(2)), None);
    let mut remaining = vec![digest(1), digest(3)];
    remaining.sort();
    assert_eq!(collect_digests(&backend, 10), remaining);

    let rewritten = backend
        .apply(&Batch::new().put_object(digest(2), payload(2)))
        .unwrap();
    assert_eq!(rewritten.objects_inserted, 1);
}

pub fn object_reads_align_with_request<H: Harness>(harness: &mut H) {
    let mut backend = create(harness);
    backend
        .apply(
            &Batch::new()
                .put_object(digest(1), payload(1))
                .put_object(digest(2), payload(2)),
        )
        .unwrap();
    let read = backend
        .get_objects(&[digest(2), digest(7), digest(1), digest(2)])
        .unwrap();
    assert_eq!(
        read,
        vec![Some(payload(2)), None, Some(payload(1)), Some(payload(2))]
    );
    assert!(backend.get_objects(&[]).unwrap().is_empty());
}

pub fn object_scan_pages_in_digest_order<H: Harness>(harness: &mut H) {
    let mut backend = create(harness);
    assert_eq!(collect_digests(&backend, 3), Vec::new());
    let batch = (1..=10).fold(Batch::new(), |batch, n| {
        batch.put_object(digest(n), payload(n))
    });
    backend.apply(&batch).unwrap();
    let mut expected = (1..=10).map(digest).collect::<Vec<_>>();
    expected.sort();
    for limit in [1, 2, 3, 5, 10, 11] {
        assert_eq!(collect_digests(&backend, limit), expected, "limit {limit}");
    }

    // A cursor that names no object resumes after its position.
    let absent = digest(100);
    let page = backend.scan_objects(Some(&absent), 20).unwrap();
    let after = expected
        .iter()
        .copied()
        .filter(|digest| *digest > absent)
        .collect::<Vec<_>>();
    assert_eq!(page.digests, after);
    assert!(!page.more);

    let last = expected.last().copied().expect("ten digests");
    let page = backend.scan_objects(Some(&last), 5).unwrap();
    assert!(page.digests.is_empty() && !page.more);
}

pub fn key_put_checks_each_precondition<H: Harness>(harness: &mut H) {
    let mut backend = create(harness);
    let r1 = revision(
        backend
            .apply(&Batch::new().put(A, b"k", b"v1", Expect::Absent))
            .unwrap(),
    );
    let v1 = KeyValue {
        value: b"v1".to_vec(),
        revision: r1,
    };
    assert_eq!(value(&backend, A, b"k"), Some(v1.clone()));

    let exists = conflict(backend.apply(&Batch::new().put(A, b"k", b"x", Expect::Absent)));
    assert_eq!(
        exists,
        Conflict {
            index: 0,
            space: A,
            key: b"k".to_vec(),
            expected: Expect::Absent,
            actual: Some(v1),
        }
    );

    let r2 = revision(
        backend
            .apply(&Batch::new().put(A, b"k", b"v2", Expect::Revision(r1)))
            .unwrap(),
    );
    assert!(r2 > r1);
    let stale = conflict(backend.apply(&Batch::new().put(A, b"k", b"x", Expect::Revision(r1))));
    assert_eq!(
        stale.actual,
        Some(KeyValue {
            value: b"v2".to_vec(),
            revision: r2,
        })
    );

    let missing =
        conflict(backend.apply(&Batch::new().put(A, b"missing", b"x", Expect::Revision(r2))));
    assert_eq!(
        (missing.key.as_slice(), missing.actual),
        (&b"missing"[..], None)
    );

    let r3 = revision(
        backend
            .apply(
                &Batch::new()
                    .put(A, b"new", b"n", Expect::Any)
                    .put(A, b"k", b"v3", Expect::Any),
            )
            .unwrap(),
    );
    assert_eq!(value(&backend, A, b"new").map(|v| v.revision), Some(r3));
    assert_eq!(
        value(&backend, A, b"k").map(|v| v.value),
        Some(b"v3".to_vec())
    );

    // The same key bytes in another space are another key.
    backend
        .apply(&Batch::new().put(B, b"k", b"other", Expect::Absent))
        .unwrap();
    assert_eq!(
        value(&backend, A, b"k").map(|v| v.value),
        Some(b"v3".to_vec())
    );

    let empty = revision(
        backend
            .apply(&Batch::new().put(LAST, Vec::new(), Vec::new(), Expect::Absent))
            .unwrap(),
    );
    assert_eq!(
        value(&backend, LAST, b""),
        Some(KeyValue {
            value: Vec::new(),
            revision: empty,
        })
    );

    let second = conflict(
        backend.apply(&Batch::new().put(A, b"fresh", b"f", Expect::Absent).put(
            A,
            b"k",
            b"x",
            Expect::Absent,
        )),
    );
    assert_eq!(second.index, 1);
    assert_eq!(value(&backend, A, b"fresh"), None);
}

pub fn key_delete_and_check_follow_preconditions<H: Harness>(harness: &mut H) {
    let mut backend = create(harness);
    let r1 = revision(
        backend
            .apply(&Batch::new().put(A, b"one", b"1", Expect::Absent))
            .unwrap(),
    );
    conflict(backend.apply(&Batch::new().delete(A, b"one", Expect::Absent)));
    conflict(backend.apply(&Batch::new().delete(A, b"one", Expect::Revision(never_issued()))));
    assert!(value(&backend, A, b"one").is_some());

    let deleted = backend
        .apply(&Batch::new().delete(A, b"one", Expect::Revision(r1)))
        .unwrap();
    assert_eq!(deleted.revision, None);
    assert_eq!(value(&backend, A, b"one"), None);
    backend
        .apply(&Batch::new().delete(A, b"one", Expect::Any))
        .unwrap();
    backend
        .apply(&Batch::new().delete(A, b"one", Expect::Absent))
        .unwrap();
    conflict(backend.apply(&Batch::new().delete(A, b"one", Expect::Revision(r1))));

    let r2 = revision(
        backend
            .apply(&Batch::new().put(A, b"two", b"2", Expect::Absent))
            .unwrap(),
    );
    let before = dump(&backend);
    let checked = backend
        .apply(&Batch::new().check(A, b"two", Expect::Revision(r2)).check(
            A,
            b"one",
            Expect::Absent,
        ))
        .unwrap();
    assert_eq!(checked, Applied::default());
    let present = conflict(backend.apply(&Batch::new().check(A, b"two", Expect::Absent)));
    assert_eq!(present.actual.map(|v| v.revision), Some(r2));
    let absent = conflict(backend.apply(&Batch::new().check(A, b"one", Expect::Revision(r2))));
    assert_eq!(absent.actual, None);
    assert_eq!(dump(&backend), before);
}

pub fn revisions_increase_and_are_never_reused<H: Harness>(harness: &mut H) {
    let mut backend = create(harness);
    let r1 = revision(
        backend
            .apply(&Batch::new().put(A, b"a", b"", Expect::Absent))
            .unwrap(),
    );
    let r2 = revision(
        backend
            .apply(&Batch::new().put(A, b"b", b"", Expect::Absent))
            .unwrap(),
    );
    assert!(r2 > r1);
    let r3 = revision(
        backend
            .apply(&Batch::new().put(A, b"c", b"", Expect::Absent).put(
                B,
                b"d",
                b"",
                Expect::Absent,
            ))
            .unwrap(),
    );
    assert!(r3 > r2);
    assert_eq!(value(&backend, A, b"c").map(|v| v.revision), Some(r3));
    assert_eq!(value(&backend, B, b"d").map(|v| v.revision), Some(r3));

    let objects_only = backend
        .apply(&Batch::new().put_object(digest(1), payload(1)))
        .unwrap();
    assert_eq!(objects_only.revision, None);
    let delete_only = backend
        .apply(&Batch::new().delete(B, b"d", Expect::Revision(r3)))
        .unwrap();
    assert_eq!(delete_only.revision, None);

    let r4 = revision(
        backend
            .apply(&Batch::new().put(B, b"d", b"", Expect::Absent))
            .unwrap(),
    );
    assert!(r4 > r3, "a recreated key gets a new revision");
    conflict(backend.apply(&Batch::new().put(B, b"d", b"", Expect::Revision(r3))));

    backend
        .apply(&Batch::new().delete(B, b"d", Expect::Revision(r4)))
        .unwrap();
    let r5 = revision(
        backend
            .apply(&Batch::new().put(A, b"e", b"", Expect::Absent))
            .unwrap(),
    );
    assert!(
        r5 > r4,
        "deleting the newest key must not free its revision"
    );
}

pub fn failing_operation_leaves_batch_without_effect<H: Harness>(harness: &mut H) {
    let mut backend = create(harness);
    let seeded = revision(
        backend
            .apply(
                &Batch::new()
                    .put_object(digest(1), payload(1))
                    .put(A, b"a", b"a0", Expect::Absent)
                    .put(A, b"b", b"b0", Expect::Absent)
                    .put(A, b"z", b"z0", Expect::Absent),
            )
            .unwrap(),
    );
    let op = |space, key: &[u8], expect, action| KeyOp {
        space,
        key: key.to_vec(),
        expect,
        action,
    };
    let valid = [
        op(A, b"c", Expect::Absent, KeyAction::Put(b"c1".to_vec())),
        op(
            A,
            b"a",
            Expect::Revision(seeded),
            KeyAction::Put(b"a1".to_vec()),
        ),
        op(A, b"b", Expect::Any, KeyAction::Delete),
        op(A, b"d", Expect::Absent, KeyAction::Check),
        op(B, b"a", Expect::Any, KeyAction::Put(b"other".to_vec())),
    ];
    let failing = [
        op(
            A,
            b"e",
            Expect::Revision(never_issued()),
            KeyAction::Put(b"e1".to_vec()),
        ),
        op(A, b"z", Expect::Absent, KeyAction::Check),
        op(A, b"z", Expect::Revision(never_issued()), KeyAction::Delete),
    ];
    let batch = |keys: Vec<KeyOp>| Batch {
        put_objects: vec![(digest(2), payload(2))],
        delete_objects: vec![digest(1)],
        keys,
    };

    let before = dump(&backend);
    for failure in &failing {
        for k in 0..=valid.len() {
            let mut keys = valid[..k].to_vec();
            keys.push(failure.clone());
            let rejected = conflict(backend.apply(&batch(keys)));
            assert_eq!(rejected.index, k);
            assert_eq!(dump(&backend), before, "operations before index {k} leaked");
        }
    }

    backend.apply(&batch(valid.to_vec())).unwrap();
    assert_eq!(object(&backend, digest(1)), None);
    assert_eq!(object(&backend, digest(2)), Some(payload(2).to_vec()));
    assert_eq!(
        value(&backend, A, b"a").map(|v| v.value),
        Some(b"a1".to_vec())
    );
    assert_eq!(value(&backend, A, b"b"), None);
    assert_eq!(
        value(&backend, A, b"c").map(|v| v.value),
        Some(b"c1".to_vec())
    );
    assert_eq!(
        value(&backend, B, b"a").map(|v| v.value),
        Some(b"other".to_vec())
    );
}

pub fn malformed_requests_are_rejected_without_effect<H: Harness>(harness: &mut H) {
    let mut backend = create(harness);
    backend
        .apply(
            &Batch::new()
                .put_object(digest(1), payload(1))
                .put(A, b"k", b"v", Expect::Absent),
        )
        .unwrap();
    let before = dump(&backend);
    for batch in [
        Batch::new()
            .put(A, b"x", b"1", Expect::Any)
            .put(A, b"x", b"2", Expect::Any),
        Batch::new()
            .put(A, b"k", b"1", Expect::Any)
            .delete(A, b"k", Expect::Any),
        Batch::new()
            .check(A, b"k", Expect::Absent)
            .put(A, b"k", b"1", Expect::Any),
        Batch::new()
            .put_object(digest(2), payload(2))
            .put_object(digest(2), payload(2)),
        Batch::new()
            .put_object(digest(1), payload(1))
            .delete_object(digest(1)),
        Batch::new().check(A, b"k", Expect::Any),
    ] {
        invalid(backend.apply(&batch));
    }
    invalid(backend.scan_keys(A, b"", None, 0));
    invalid(backend.scan_objects(None, 0));
    assert_eq!(dump(&backend), before);

    assert_eq!(backend.apply(&Batch::new()).unwrap(), Applied::default());
    backend
        .apply(&Batch::new().put(A, b"same", b"1", Expect::Absent).put(
            B,
            b"same",
            b"2",
            Expect::Absent,
        ))
        .unwrap();
}

pub fn key_scan_orders_filters_and_resumes<H: Harness>(harness: &mut H) {
    let mut backend = create(harness);
    let keys: [&[u8]; 10] = [
        &[0xFF, 0xFF],
        &[0, 1],
        &[],
        &[1, 0],
        &[0, 0xFF, 0xFF],
        &[0],
        &[0xFF],
        &[0, 0],
        &[1],
        &[0, 0xFF],
    ];
    let mut batch = Batch::new();
    for key in keys {
        batch = batch.put(A, key, [b"a".as_slice(), key].concat(), Expect::Absent);
    }
    let other_keys: [&[u8]; 3] = [&[0], &[0, 0], &[2]];
    for key in other_keys {
        batch = batch.put(B, key, [b"b".as_slice(), key].concat(), Expect::Absent);
    }
    let written = revision(backend.apply(&batch).unwrap());
    let mut sorted = keys.map(<[u8]>::to_vec);
    sorted.sort();

    let prefixes: [&[u8]; 8] = [
        &[],
        &[0],
        &[0, 0xFF],
        &[0xFF],
        &[0xFF, 0xFF],
        &[0xFF, 0xFF, 0xFF],
        &[2],
        &[0, 2],
    ];
    for prefix in prefixes {
        let expected = sorted
            .iter()
            .filter(|key| key.starts_with(prefix))
            .map(|key| KeyEntry {
                key: key.clone(),
                value: [b"a".as_slice(), key].concat(),
                revision: written,
            })
            .collect::<Vec<_>>();
        for limit in 1..=4 {
            assert_eq!(
                collect_keys(&backend, A, prefix, None, limit),
                expected,
                "prefix {prefix:?}, limit {limit}"
            );
        }
    }

    let scanned_keys = |prefix: &[u8], after: &[u8]| {
        let page = backend.scan_keys(A, prefix, Some(after), 10).unwrap();
        assert!(!page.more);
        page.entries
            .into_iter()
            .map(|entry| entry.key)
            .collect::<Vec<_>>()
    };
    assert_eq!(scanned_keys(&[1], &[0, 0xFF]), vec![vec![1], vec![1, 0]]);
    assert_eq!(
        scanned_keys(&[0], &[0, 0, 5]),
        vec![vec![0, 1], vec![0, 0xFF], vec![0, 0xFF, 0xFF]]
    );
    assert_eq!(scanned_keys(&[0], &[0, 0xFF, 0xFF]), Vec::<Vec<u8>>::new());
    assert_eq!(scanned_keys(&[0], &[1]), Vec::<Vec<u8>>::new());

    let other = collect_keys(&backend, B, b"", None, 2)
        .into_iter()
        .map(|entry| entry.key)
        .collect::<Vec<_>>();
    assert_eq!(other, vec![vec![0], vec![0, 0], vec![2]]);
    assert!(collect_keys(&backend, LAST, b"", None, 2).is_empty());
}

pub fn limits_reject_requests_without_effect<H: Harness>(harness: &mut H) {
    let mut backend = create(harness);
    let limits = backend.capabilities().limits;
    backend
        .apply(&Batch::new().put(A, b"seed", b"", Expect::Absent))
        .unwrap();
    let before = dump(&backend);
    let size = |bytes: u64| usize::try_from(bytes).expect("limit fits in memory");

    let long_key = vec![7; size(limits.max_key_bytes + 1)];
    limit(
        backend.apply(&Batch::new().put(A, long_key.clone(), b"", Expect::Any)),
        Limit::KeyBytes,
    );
    limit(
        backend.apply(&Batch::new().check(A, long_key.clone(), Expect::Absent)),
        Limit::KeyBytes,
    );
    limit(backend.read_key(A, &long_key), Limit::KeyBytes);
    limit(backend.scan_keys(A, &long_key, None, 1), Limit::KeyBytes);

    let big: Arc<[u8]> = vec![0; size(limits.max_value_bytes + 1)].into();
    limit(
        backend.apply(&Batch::new().put_object(digest(1), Arc::clone(&big))),
        Limit::ValueBytes,
    );
    limit(
        backend.apply(&Batch::new().put(A, b"big", big.to_vec(), Expect::Any)),
        Limit::ValueBytes,
    );
    drop(big);

    let many = (0..=limits.max_batch_ops).fold(Batch::new(), |batch, n| {
        batch.put(A, n.to_be_bytes(), b"", Expect::Any)
    });
    limit(backend.apply(&many), Limit::BatchOps);

    // Objects share one buffer, so the batch exceeds its byte limit without that much memory.
    let chunk = limits.max_value_bytes.min(limits.max_batch_bytes).max(1);
    let count = limits.max_batch_bytes / chunk + 1;
    if count <= limits.max_batch_ops {
        let shared: Arc<[u8]> = vec![0; size(chunk)].into();
        let heavy = (0..count).fold(Batch::new(), |batch, n| {
            batch.put_object(digest(1000 + n), Arc::clone(&shared))
        });
        limit(backend.apply(&heavy), Limit::BatchBytes);
    }

    let read_limit = u32::try_from(limits.max_read_items).expect("read limit fits a page size");
    let too_many = (0..=limits.max_read_items).map(digest).collect::<Vec<_>>();
    limit(backend.get_objects(&too_many), Limit::ReadItems);
    if let Some(over) = read_limit.checked_add(1) {
        limit(backend.scan_keys(A, b"", None, over), Limit::ReadItems);
        limit(backend.scan_objects(None, over), Limit::ReadItems);
    }
    assert_eq!(dump(&backend), before);

    let longest = vec![7; size(limits.max_key_bytes)];
    let largest: Arc<[u8]> = vec![1; size(limits.max_value_bytes)].into();
    backend
        .apply(
            &Batch::new()
                .put(A, longest.clone(), b"", Expect::Absent)
                .put_object(digest(1), Arc::clone(&largest)),
        )
        .unwrap();
    assert!(value(&backend, A, &longest).is_some());
    assert_eq!(backend.get_object(&digest(1)).unwrap(), Some(largest));
    let at_limit = (0..limits.max_read_items).map(digest).collect::<Vec<_>>();
    assert_eq!(
        backend.get_objects(&at_limit).unwrap().len(),
        at_limit.len()
    );
    backend.scan_keys(A, b"", None, read_limit).unwrap();
    backend.scan_objects(None, read_limit).unwrap();
}

pub fn reopened_store_keeps_state_and_revisions<H: Harness>(harness: &mut H) {
    let mut backend = create(harness);
    let capabilities = backend.capabilities();
    // Evictable stores survive restarts too; only the platform may drop them whole.
    if capabilities.durability == Durability::Memory {
        return;
    }
    backend
        .apply(
            &Batch::new()
                .put_object(digest(1), payload(1))
                .put_object(digest(2), payload(2))
                .put(A, b"kept", b"1", Expect::Absent)
                .put(B, b"kept", b"2", Expect::Absent),
        )
        .unwrap();
    let newest = revision(
        backend
            .apply(&Batch::new().put(A, b"newest", b"3", Expect::Absent))
            .unwrap(),
    );
    backend
        .apply(
            &Batch::new()
                .delete(A, b"newest", Expect::Revision(newest))
                .delete_object(digest(2)),
        )
        .unwrap();
    let before = dump(&backend);

    let mut reopened = harness
        .reopen(backend)
        .expect("the harness of a persistent backend must reopen it");
    assert_eq!(reopened.capabilities(), capabilities);
    assert_eq!(dump(&reopened), before);
    let next = revision(
        reopened
            .apply(&Batch::new().put(A, b"after", b"4", Expect::Absent))
            .unwrap(),
    );
    assert!(next > newest, "the revision counter must survive reopening");
}

pub fn concurrent_handles_race_for_one_cas<H: Harness>(harness: &mut H) {
    const WRITERS: u8 = 4;
    const INCREMENTS: u64 = 5;

    let mut first = create(harness);
    if !first.capabilities().concurrent_writers() {
        return;
    }
    let mut connect = |backend: &H::Backend| {
        harness
            .connect(backend)
            .expect("the harness of a concurrent backend must connect handles")
    };
    let mut second = connect(&first);

    let r1 = revision(
        first
            .apply(&Batch::new().put(A, b"k", b"v1", Expect::Absent))
            .unwrap(),
    );
    assert_eq!(value(&second, A, b"k").map(|v| v.revision), Some(r1));
    let won = revision(
        second
            .apply(&Batch::new().put(A, b"k", b"v2", Expect::Revision(r1)))
            .unwrap(),
    );
    let lost = conflict(first.apply(&Batch::new().put(A, b"k", b"v3", Expect::Revision(r1))));
    assert_eq!(
        lost.actual,
        Some(KeyValue {
            value: b"v2".to_vec(),
            revision: won,
        })
    );

    let handles = (0..WRITERS).map(|_| connect(&first)).collect::<Vec<_>>();
    let barrier = Barrier::new(handles.len());
    let created = thread::scope(|scope| {
        let workers = handles
            .into_iter()
            .zip(0..WRITERS)
            .map(|(mut handle, writer)| {
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    let created = loop {
                        match handle.apply(&Batch::new().put(A, b"race", [writer], Expect::Absent))
                        {
                            Err(StorageError::Busy) => continue,
                            other => break other,
                        }
                    };
                    for _ in 0..INCREMENTS {
                        increment(&mut handle);
                    }
                    created.map(|_| writer)
                })
            })
            .collect::<Vec<_>>();
        workers
            .into_iter()
            .map(|worker| worker.join().expect("writer thread"))
            .collect::<Vec<_>>()
    });

    let winners = created
        .iter()
        .filter_map(|result| result.as_ref().ok().copied())
        .collect::<Vec<_>>();
    assert_eq!(
        winners.len(),
        1,
        "exactly one CAS on an absent key wins: {created:?}"
    );
    for result in &created {
        assert!(
            matches!(result, Ok(_) | Err(StorageError::Conflict(_))),
            "{result:?}"
        );
    }
    assert_eq!(
        value(&first, A, b"race").map(|v| v.value),
        Some(vec![winners[0]])
    );
    let counter = value(&first, A, b"counter").expect("counter written").value;
    assert_eq!(
        counter,
        (u64::from(WRITERS) * INCREMENTS).to_be_bytes(),
        "revision CAS loses no update"
    );
}

/// Read-modify-write with a revision CAS, retried until it wins.
fn increment(backend: &mut impl StorageBackend) {
    loop {
        let current = backend.read_key(A, b"counter").unwrap();
        let (next, expect) = match current {
            Some(current) => {
                let bytes: [u8; 8] = current.value.as_slice().try_into().expect("u64 counter");
                (
                    u64::from_be_bytes(bytes) + 1,
                    Expect::Revision(current.revision),
                )
            }
            None => (1, Expect::Absent),
        };
        match backend.apply(&Batch::new().put(A, b"counter", next.to_be_bytes(), expect)) {
            Ok(_) => return,
            Err(StorageError::Conflict(_) | StorageError::Busy) => {}
            Err(error) => panic!("increment failed: {error}"),
        }
    }
}
