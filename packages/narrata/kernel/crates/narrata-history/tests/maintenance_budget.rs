#![allow(clippy::unwrap_used)]
#[macro_use]
mod support;

use narrata_history::{
    DomainKinds, History, MaintenanceBackend, MaintenanceBudget, RetentionPolicy,
};
use narrata_storage::StorageBackend;
use support::{Fixture, chain, open};

on_both_backends!(bounded_scans_and_dry_run);
on_both_backends!(
    oversized_payloads_fail_bounded_reads,
    wrong_kind_cannot_prove_a_leaf,
    corrupt_destination_preserves_root
);

fn oversized_payloads_fail_bounded_reads<F: Fixture>() {
    use narrata_storage::{Batch, Expect, KeySpace, ObjectDigest};
    let f = F::new();
    let mut store = f.backend();
    let id = ObjectDigest::from_bytes([7; 32]);
    let space = KeySpace::new(9);
    store
        .apply(&Batch::new().put_object(id, vec![0; 8192]).put(
            space,
            b"a",
            vec![0; 8192],
            Expect::Absent,
        ))
        .unwrap();
    assert!(store.get_objects_bounded(&[id], 1024).is_err());
    assert!(store.read_key_bounded(space, b"a", 1024).is_err());
    assert!(store.scan_keys_bounded(space, b"", None, 1, 1024).is_err());
    assert_eq!(
        store.get_objects_bounded(&[id], 8192).unwrap()[0]
            .as_ref()
            .unwrap()
            .len(),
        8192
    );
    assert_eq!(
        store
            .scan_keys_bounded(space, b"", None, 1, 8193)
            .unwrap()
            .entries
            .len(),
        1
    );
}

fn wrong_kind_cannot_prove_a_leaf<F: Fixture>() {
    use narrata_history::{
        Commit, Domain, Object, RefKey, RefMutation, Transaction, testing::COUNTER_COMMIT_KIND,
    };
    use narrata_storage::Batch;
    let f = F::new();
    let mut history = open(&f);
    let (_, commits) = chain(&mut history, "kept", &[1]);
    let b = commits[1];
    let bad = Commit::root(support::COUNTER.artifact_id(), b).to_object(COUNTER_COMMIT_KIND);
    // Bypass checked writes to reproduce on-disk graph damage: A claims commit B is a state.
    f.backend()
        .apply(&Batch::new().put_object(
            narrata_storage::ObjectDigest::from_bytes(*bad.id().as_bytes()),
            bad.bytes().to_vec(),
        ))
        .unwrap();
    history
        .write_atomic(
            &Transaction {
                refs: vec![RefMutation {
                    key: RefKey::save(support::name("bad"), support::name("slot")),
                    expected: None,
                    next: Some(bad.id()),
                }],
                ..Default::default()
            },
            |_| Ok(Vec::new()),
        )
        .unwrap();
    let before = f.backend().scan_objects(None, 1024).unwrap();
    let issues = history.integrity_scan().unwrap();
    assert!(issues.iter().any(|i| i.object == bad.id()));
    assert!(
        history
            .collect_atomic(RetentionPolicy {
                now: 100,
                grace_seconds: 0,
                dry_run: false
            })
            .is_err()
    );
    assert_eq!(before, f.backend().scan_objects(None, 1024).unwrap());
    let _: Option<Object> = history.get_object(b).unwrap();
}

fn corrupt_destination_preserves_root<F: Fixture>() {
    use narrata_history::{BundleLimits, RefKey};
    use narrata_storage::{Batch, ObjectDigest};
    let f = F::new();
    let mut history = open(&f);
    let (_, a) = chain(&mut history, "active", &[1]);
    let target = RefKey::active(support::name("active")).unwrap();
    let old = history.read_ref(&target).unwrap().unwrap();
    let source = support::Memory::new();
    let mut incoming = open(&source);
    let (_, b) = chain(&mut incoming, "incoming", &[2]);
    let bundle = incoming.export(b[1], &Default::default()).unwrap();
    for id in [
        b[1],
        incoming.load(&support::COUNTER, b[1]).unwrap().header.state,
    ] {
        f.backend()
            .apply(&Batch::new().put_object(
                ObjectDigest::from_bytes(*id.as_bytes()),
                b"damaged".to_vec(),
            ))
            .unwrap();
        assert!(
            history
                .import(
                    &support::COUNTER,
                    &bundle.to_bytes().unwrap(),
                    BundleLimits::default(),
                    target.clone(),
                    Some(old.revision),
                    100
                )
                .is_err()
        );
        assert_eq!(history.read_ref(&target).unwrap().unwrap(), old);
        assert_eq!(open(&f).load(&support::COUNTER, a[1]).unwrap().state, 1);
        f.backend()
            .apply(&Batch::new().delete_object(ObjectDigest::from_bytes(*id.as_bytes())))
            .unwrap();
    }
}

#[test]
fn malformed_manifest_count_reaches_decoder_with_positive_budget() {
    use narrata_history::Object;
    use narrata_kernel::codec::CborWriter;
    use narrata_storage::{Batch, MemoryBackend, ObjectDigest};
    let store = MemoryBackend::new();
    let history = History::open(
        store.share(),
        DomainKinds::<narrata_history::testing::Counter>::new(),
    )
    .unwrap();
    let mut writer = CborWriter::new();
    writer.map(4);
    writer.unsigned(0);
    writer.unsigned(1);
    writer.unsigned(1);
    writer.bytes(&[0; 32]);
    writer.unsigned(2);
    writer.array(1_000_000);
    let bad = Object::new(7, 1, &writer.into_bytes());
    store
        .share()
        .apply(&Batch::new().put_object(
            ObjectDigest::from_bytes(*bad.id().as_bytes()),
            bad.bytes().to_vec(),
        ))
        .unwrap();
    let bounded = History::open_existing(
        MaintenanceBackend::new(
            store.share(),
            MaintenanceBudget {
                object_reads: 1,
                bytes: 1024,
                pages: 1,
                work: 1024,
                page_size: 1,
            },
        )
        .unwrap(),
        DomainKinds::<narrata_history::testing::Counter>::new(),
    )
    .unwrap();
    let issues = bounded.integrity_scan().unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].object, bad.id());
    assert!(issues[0].diagnostic.contains("end"));
    drop(history);
}

#[test]
fn wide_valid_manifest_consumes_encoded_byte_work_before_expansion() {
    use narrata_history::{CheckpointManifest, Descriptor, ObjectId};
    let manifest = CheckpointManifest {
        root: ObjectId::from_bytes([0; 32]),
        host_manifest: None,
        objects: (0_u32..256)
            .map(|i| {
                let mut id = [0; 32];
                id[..4].copy_from_slice(&i.to_be_bytes());
                Descriptor {
                    id: ObjectId::from_bytes(id),
                    kind: 7,
                    schema: 1,
                    bytes: 10,
                }
            })
            .collect(),
    };
    let object = manifest.to_object().unwrap();
    assert!(CheckpointManifest::decode(object.payload()).is_ok());
    use narrata_storage::{Batch, MemoryBackend, ObjectDigest};
    let store = MemoryBackend::new();
    History::open(
        store.share(),
        DomainKinds::<narrata_history::testing::Counter>::new(),
    )
    .unwrap();
    store
        .share()
        .apply(&Batch::new().put_object(
            ObjectDigest::from_bytes(*object.id().as_bytes()),
            object.bytes().to_vec(),
        ))
        .unwrap();
    let bounded = History::open_existing(
        MaintenanceBackend::new(
            store.share(),
            MaintenanceBudget {
                object_reads: 10,
                bytes: 100_000,
                pages: 1,
                work: 1024,
                page_size: 1,
            },
        )
        .unwrap(),
        DomainKinds::<narrata_history::testing::Counter>::new(),
    )
    .unwrap();
    assert!(bounded.integrity_scan().is_err());
}

fn budget() -> MaintenanceBudget {
    MaintenanceBudget {
        object_reads: 10_000,
        bytes: 1_000_000,
        pages: 1000,
        work: 100_000,
        page_size: 2,
    }
}

#[test]
fn positive_small_budget_never_invokes_an_unbounded_legacy_read() {
    use narrata_storage::*;
    struct Legacy;
    impl StorageBackend for Legacy {
        fn capabilities(&self) -> Capabilities {
            MemoryBackend::new().capabilities()
        }
        fn get_objects(
            &self,
            _: &[ObjectDigest],
        ) -> Result<Vec<Option<std::sync::Arc<[u8]>>>, StorageError> {
            unreachable!("oversized object read was invoked")
        }
        fn scan_objects(
            &self,
            _: Option<&ObjectDigest>,
            _: u32,
        ) -> Result<ObjectPage, StorageError> {
            unreachable!("oversized digest scan was invoked")
        }
        fn read_key(&self, _: KeySpace, _: &[u8]) -> Result<Option<KeyValue>, StorageError> {
            unreachable!("oversized key read was invoked")
        }
        fn scan_keys(
            &self,
            _: KeySpace,
            _: &[u8],
            _: Option<&[u8]>,
            _: u32,
        ) -> Result<KeyPage, StorageError> {
            unreachable!("oversized key scan was invoked")
        }
        fn apply(&mut self, _: &Batch) -> Result<Applied, StorageError> {
            unreachable!("read-only test")
        }
    }
    let bounded = MaintenanceBackend::new(
        Legacy,
        MaintenanceBudget {
            bytes: 1024,
            work: 4096,
            ..budget()
        },
    )
    .unwrap();
    assert!(
        bounded
            .get_objects(&[ObjectDigest::from_bytes([0; 32])])
            .is_err()
    );
    assert!(bounded.read_key(KeySpace::new(0), b"k").is_err());
    assert!(bounded.scan_keys(KeySpace::new(0), b"", None, 1).is_err());
}

#[test]
fn atomic_sweep_rejects_the_whole_plan_before_any_deletion() {
    use narrata_history::{RefMutation, RefScope, Transaction};
    let backend = narrata_storage::MemoryBackend::with_limits(narrata_storage::Limits {
        max_batch_ops: 20,
        ..narrata_storage::Limits::DEFAULT
    });
    let mut history = History::open(backend.share(), DomainKinds::new()).unwrap();
    chain(&mut history, "unrooted", &[1, 2, 3, 4, 5]);
    let roots = history.scan_refs(&RefScope::All, None, 100).unwrap();
    history
        .write_atomic(
            &Transaction {
                refs: roots
                    .items
                    .into_iter()
                    .map(|(key, value)| RefMutation {
                        key,
                        expected: Some(value.revision),
                        next: None,
                    })
                    .collect(),
                ..Transaction::default()
            },
            |_| Ok(Vec::new()),
        )
        .unwrap();
    let before = backend.scan_objects(None, 1024).unwrap();
    let mut bounded = History::open_existing(
        MaintenanceBackend::new(backend.share(), budget()).unwrap(),
        DomainKinds::<narrata_history::testing::Counter>::new(),
    )
    .unwrap();
    let policy = RetentionPolicy {
        now: 100,
        grace_seconds: 0,
        dry_run: false,
    };
    assert!(bounded.collect_atomic(policy).is_err());
    assert_eq!(backend.scan_objects(None, 1024).unwrap(), before);
    assert!(history.integrity_scan().unwrap().is_empty());
    // The pre-existing background API still supports its ordered fenced batches.
    assert!(!history.collect(policy).unwrap().removed.is_empty());
}

fn bounded_scans_and_dry_run<F: Fixture>() {
    let fixture = F::new();
    let mut history = open(&fixture);
    chain(&mut history, "kept", &[1, 2, 3, 4]);
    chain(&mut history, "discarded", &[5, 6]);
    let discarded = history
        .scan_refs(&narrata_history::RefScope::All, None, 100)
        .unwrap()
        .items
        .into_iter()
        .filter(|(key, _)| key.owner().as_str() == "discarded")
        .map(|(key, value)| narrata_history::RefMutation {
            key,
            expected: Some(value.revision),
            next: None,
        })
        .collect();
    history
        .write_atomic(
            &narrata_history::Transaction {
                refs: discarded,
                ..Default::default()
            },
            |_| Ok(Vec::new()),
        )
        .unwrap();
    let before = fixture.backend().scan_objects(None, 1024).unwrap();
    for bound in [
        MaintenanceBudget {
            object_reads: 0,
            ..budget()
        },
        MaintenanceBudget {
            pages: 0,
            ..budget()
        },
        MaintenanceBudget {
            bytes: 0,
            ..budget()
        },
        MaintenanceBudget {
            work: 0,
            ..budget()
        },
        MaintenanceBudget {
            object_reads: 2,
            ..budget()
        },
        MaintenanceBudget {
            bytes: 128,
            ..budget()
        },
        MaintenanceBudget {
            pages: 2,
            ..budget()
        },
        MaintenanceBudget {
            work: 8,
            ..budget()
        },
    ] {
        let mut bounded = History::open_existing(
            MaintenanceBackend::new(fixture.backend(), bound).unwrap(),
            DomainKinds::<narrata_history::testing::Counter>::new(),
        );
        if let Ok(history) = &mut bounded {
            assert!(
                history
                    .collect(RetentionPolicy {
                        now: 100,
                        grace_seconds: 0,
                        dry_run: false
                    })
                    .is_err()
            );
        }
        assert_eq!(fixture.backend().scan_objects(None, 1024).unwrap(), before);
    }
    let mut bounded = History::open_existing(
        MaintenanceBackend::new(fixture.backend(), budget()).unwrap(),
        DomainKinds::<narrata_history::testing::Counter>::new(),
    )
    .unwrap();
    assert!(bounded.integrity_scan().unwrap().is_empty());
    let report = bounded
        .collect(RetentionPolicy {
            now: 100,
            grace_seconds: 0,
            dry_run: true,
        })
        .unwrap();
    assert!(report.dry_run);
    assert!(report.reachable > 0);
    assert!(!report.removed.is_empty());
    assert_eq!(fixture.backend().scan_objects(None, 1024).unwrap(), before);
}
