#![allow(clippy::unwrap_used)]
#[macro_use]
mod support;

use narrata_history::{
    DomainKinds, History, MaintenanceBackend, MaintenanceBudget, RetentionPolicy,
};
use narrata_storage::StorageBackend;
use support::{Fixture, chain, open};

on_both_backends!(bounded_scans_and_dry_run);

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
