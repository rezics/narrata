#![allow(clippy::unwrap_used)]
mod support;

use narrata_history::{
    DomainKinds, History, MaintenanceBudget, Object, Pin, RefMutation, RefScope, RetentionPolicy,
    Transaction,
};
use narrata_nodes::history::{NodeDomain, maintenance};
use narrata_storage::{Batch, ObjectDigest, StorageBackend};

fn budget() -> MaintenanceBudget {
    MaintenanceBudget {
        object_reads: 10_000,
        bytes: 10_000_000,
        pages: 1000,
        work: 100_000,
        page_size: 2,
    }
}

#[test]
fn observed_root_cas_and_expiring_pins_preserve_their_closure() {
    let (mut session, names) = support::session(&support::compiled());
    support::choose(&mut session, &names, &["wave"]).unwrap();
    let cursor = narrata_history::ObjectId::from_bytes(*session.cursor().unwrap().as_bytes());
    let backend = session.into_backend();
    let mut raw =
        History::open_existing(backend.share(), DomainKinds::<NodeDomain>::new()).unwrap();
    let roots = raw.scan_refs(&RefScope::All, None, 1024).unwrap().items;
    raw.write(
        &Transaction {
            pins: vec![Pin {
                owner: "operator".into(),
                object: cursor,
                expires_at: Some(100),
            }],
            refs: roots
                .iter()
                .map(|(key, value)| RefMutation {
                    key: key.clone(),
                    expected: Some(value.revision),
                    next: None,
                })
                .collect(),
            ..Transaction::default()
        },
        |_| Ok(Vec::new()),
    )
    .unwrap();
    let mut maintained = maintenance::open(backend.share(), budget()).unwrap();
    let report = maintenance::collect(
        &mut maintained,
        RetentionPolicy {
            now: 99,
            grace_seconds: 0,
            dry_run: false,
        },
    )
    .unwrap();
    assert!(report.removed.is_empty());
    assert!(!report.retained.is_empty());
    let pins = maintained.scan_pins(None, 2).unwrap();
    assert_eq!(pins.items[0].expires_at, Some(100));
    let mut maintained = maintenance::open(backend.share(), budget()).unwrap();
    let dry = maintenance::collect(
        &mut maintained,
        RetentionPolicy {
            now: 100,
            grace_seconds: 0,
            dry_run: true,
        },
    )
    .unwrap();
    assert!(!dry.removed.is_empty());
    assert!(raw.get_object(cursor).unwrap().is_some());
    let mut maintained = maintenance::open(backend, budget()).unwrap();
    let report = maintenance::collect(
        &mut maintained,
        RetentionPolicy {
            now: 100,
            grace_seconds: 0,
            dry_run: false,
        },
    )
    .unwrap();
    assert_eq!(report.removed, dry.removed);
    assert!(raw.get_object(cursor).unwrap().is_none());
}

#[test]
fn unknown_kinds_and_limits_refuse_destructive_work_without_deleting_candidates() {
    let (session, _) = support::session(&support::compiled());
    let backend = session.into_backend();
    let mut raw =
        History::open_existing(backend.share(), DomainKinds::<NodeDomain>::new()).unwrap();
    let roots = raw.scan_refs(&RefScope::All, None, 1024).unwrap().items;
    let mutations = roots
        .iter()
        .map(|(key, value)| RefMutation {
            key: key.clone(),
            expected: Some(value.revision),
            next: None,
        })
        .collect();
    raw.write(
        &Transaction {
            refs: mutations,
            ..Transaction::default()
        },
        |_| Ok(Vec::new()),
    )
    .unwrap();
    let unknown = Object::new(0x9900, 1, &[0xf6]);
    backend
        .share()
        .apply(&Batch::new().put_object(
            ObjectDigest::from_bytes(*unknown.id().as_bytes()),
            unknown.bytes(),
        ))
        .unwrap();
    let before = backend.scan_objects(None, 1024).unwrap();
    let mut maintained = maintenance::open(backend.share(), budget()).unwrap();
    assert_eq!(
        maintenance::collect(
            &mut maintained,
            RetentionPolicy {
                now: 100,
                grace_seconds: 0,
                dry_run: false
            }
        )
        .unwrap_err()
        .code,
        "unregistered_kind"
    );
    assert_eq!(backend.scan_objects(None, 1024).unwrap(), before);
    let mut maintained = maintenance::open(
        backend.share(),
        MaintenanceBudget {
            pages: 0,
            ..budget()
        },
    )
    .unwrap();
    assert_eq!(
        maintenance::collect(
            &mut maintained,
            RetentionPolicy {
                now: 100,
                grace_seconds: 0,
                dry_run: false
            }
        )
        .unwrap_err()
        .code,
        "limit"
    );
    assert_eq!(backend.scan_objects(None, 1024).unwrap(), before);
    assert_eq!(
        maintenance::collect(&mut maintained, RetentionPolicy::default())
            .unwrap_err()
            .code,
        "limit"
    );
}
