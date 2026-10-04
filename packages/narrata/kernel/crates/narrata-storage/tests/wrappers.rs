#![allow(clippy::unwrap_used)]

use narrata_storage::{
    Batch, Expect, KeySpace, MemoryBackend, ObjectDigest, StorageBackend, StorageError,
    testing::{Counting, Counts, Fault, FaultInjecting, FaultPlan, Primitive, Trigger},
};

const SPACE: KeySpace = KeySpace::new(3);

fn digest(n: u8) -> ObjectDigest {
    ObjectDigest::from_bytes([n; 32])
}

#[test]
fn fault_before_a_call_has_no_effect() {
    let plan = FaultPlan::new().at(
        Trigger::Nth(Primitive::Apply, 1),
        Fault::Before(StorageError::Busy),
    );
    let mut backend = FaultInjecting::new(MemoryBackend::new(), plan);
    let batch = Batch::new().put(SPACE, b"k", b"v", Expect::Absent);
    assert_eq!(backend.apply(&batch), Err(StorageError::Busy));
    assert_eq!(backend.read_key(SPACE, b"k").unwrap(), None);
    backend.apply(&batch).unwrap();
    assert!(backend.read_key(SPACE, b"k").unwrap().is_some());
}

#[test]
fn fault_after_a_call_hides_an_applied_batch_behind_an_unknown_outcome() {
    let plan = FaultPlan::new().at(Trigger::Nth(Primitive::Apply, 1), Fault::After);
    let mut backend = FaultInjecting::new(MemoryBackend::new(), plan);
    let error = backend
        .apply(&Batch::new().put(SPACE, b"k", b"v", Expect::Absent))
        .unwrap_err();
    assert!(error.outcome_unknown(), "{error}");
    assert!(backend.read_key(SPACE, b"k").unwrap().is_some());
}

#[test]
fn faults_select_calls_by_overall_or_per_primitive_position() {
    let plan = FaultPlan::new()
        .at(
            Trigger::Call(2),
            Fault::Before(StorageError::Io("disk".to_owned())),
        )
        .at(Trigger::Nth(Primitive::ScanKeys, 1), Fault::After);
    let backend = FaultInjecting::new(MemoryBackend::new(), plan);
    assert_eq!(backend.read_key(SPACE, b"k").unwrap(), None);
    assert_eq!(
        backend.get_objects(&[digest(1)]),
        Err(StorageError::Io("disk".to_owned()))
    );
    assert!(
        backend
            .scan_keys(SPACE, b"", None, 1)
            .unwrap_err()
            .outcome_unknown()
    );
    assert!(backend.scan_keys(SPACE, b"", None, 1).is_ok());
    assert_eq!(backend.calls(), 4);
}

/// The pattern for engine fault tests: count the calls an operation makes, fail each one in
/// turn before and after it runs, and check that state is all or nothing.
#[test]
fn failing_each_call_in_turn_never_publishes_partial_state() {
    fn publish(backend: &mut impl StorageBackend) -> Result<(), StorageError> {
        let current = backend.read_key(SPACE, b"root")?;
        let expect = current.map_or(Expect::Absent, |value| Expect::Revision(value.revision));
        backend.apply(
            &Batch::new()
                .put_object(digest(1), vec![1, 2, 3])
                .put(SPACE, b"index", b"i", Expect::Any)
                .put(SPACE, b"root", b"r", expect),
        )?;
        Ok(())
    }

    let mut probe = FaultInjecting::new(MemoryBackend::new(), FaultPlan::new());
    publish(&mut probe).unwrap();
    let calls = probe.calls();
    assert_eq!(calls, 2);
    for call in 1..=calls {
        for fault in [Fault::Before(StorageError::Busy), Fault::After] {
            let plan = FaultPlan::new().at(Trigger::Call(call), fault.clone());
            let mut backend = FaultInjecting::new(MemoryBackend::new(), plan);
            let error = publish(&mut backend).unwrap_err();
            let root = backend.read_key(SPACE, b"root").unwrap().is_some();
            let index = backend.read_key(SPACE, b"index").unwrap().is_some();
            let object = backend.get_object(&digest(1)).unwrap().is_some();
            assert_eq!((index, object), (root, root), "call {call}, {fault:?}");
            assert!(!root || error.outcome_unknown(), "call {call}, {fault:?}");
        }
    }
}

#[test]
fn counting_records_calls_and_returned_items() {
    let mut backend = Counting::new(MemoryBackend::new());
    let batch = (0..5_u8)
        .fold(Batch::new(), |batch, n| {
            batch.put(SPACE, [n], [n], Expect::Absent)
        })
        .put_object(digest(1), vec![1])
        .put_object(digest(2), vec![2]);
    backend.apply(&batch).unwrap();
    backend
        .get_objects(&[digest(1), digest(2), digest(3)])
        .unwrap();
    let first = backend.scan_keys(SPACE, b"", None, 3).unwrap();
    backend
        .scan_keys(SPACE, b"", first.resume_after(), 3)
        .unwrap();
    backend.scan_objects(None, 10).unwrap();
    backend.read_key(SPACE, &[0]).unwrap();

    assert_eq!(
        backend.take_counts(),
        Counts {
            get_objects: 1,
            scan_objects: 1,
            read_key: 1,
            scan_keys: 2,
            apply: 1,
            objects_returned: 2,
            objects_scanned: 2,
            keys_scanned: 5,
        }
    );
    assert_eq!(backend.counts(), Counts::default());
}
