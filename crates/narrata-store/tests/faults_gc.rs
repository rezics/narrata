#![allow(clippy::panic, clippy::unwrap_used)]

use std::{collections::BTreeSet, sync::Arc};

use narrata_core::{
    ExecutionId, InputId,
    program::{encode_program_artifact, load_program},
    runtime::CheckedRuntimeInput,
};
use narrata_store::{
    BranchId, CheckpointBundle, FaultPoint, InitialRecordingMode, MemoryStore, RefKey, RefName,
    SaveStore, SessionCoordinator,
};
use narrata_testkit::generator::branch_call_choice_v0;

fn coordinator() -> SessionCoordinator<MemoryStore> {
    let program = load_program(
        &encode_program_artifact(&branch_call_choice_v0()),
        &Default::default(),
    )
    .unwrap();
    SessionCoordinator::create(
        MemoryStore::new(),
        Arc::clone(&program),
        ExecutionId::from_u128(1),
        RefName::new("session").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Complete,
        1,
    )
    .unwrap()
}

#[test]
fn injected_failures_never_publish_partial_commit() {
    for point in [
        FaultPoint::SnapshotWrite,
        FaultPoint::ReceiptWrite,
        FaultPoint::CommitWrite,
        FaultPoint::EdgeIndexWrite,
        FaultPoint::RefCas,
        FaultPoint::CatalogEventWrite,
        FaultPoint::CatalogHeadCas,
        FaultPoint::TransactionCommit,
    ] {
        let mut coordinator = coordinator();
        let before_objects = coordinator.store().list_objects().unwrap();
        let before_refs = coordinator.store().list_refs().unwrap();
        let before_catalogs = coordinator.store().list_catalog_heads().unwrap();
        coordinator.store_mut().inject_fault(Some(point));
        assert!(
            coordinator
                .dispatch(
                    CheckedRuntimeInput::start(InputId::from_u128(1)),
                    Default::default(),
                    2,
                )
                .is_err()
        );
        assert_eq!(coordinator.store().list_objects().unwrap(), before_objects);
        assert_eq!(coordinator.store().list_refs().unwrap(), before_refs);
        assert_eq!(
            coordinator.store().list_catalog_heads().unwrap(),
            before_catalogs
        );
    }
}

#[test]
fn gc_dry_run_matches_sweep_and_keeps_root_closure() {
    let mut coordinator = coordinator();
    coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap();
    let rooted = coordinator.store().list_objects().unwrap().len();
    let dry = coordinator
        .store_mut()
        .collect(narrata_store::RetentionPolicy {
            now: 100,
            grace_seconds: 0,
            dry_run: true,
        })
        .unwrap();
    assert_eq!(coordinator.store().list_objects().unwrap().len(), rooted);
    assert_eq!(
        dry.removed.values().map(|value| value.objects).sum::<u64>(),
        0
    );

    coordinator.delete_complete_recording(3).unwrap();
    let sweep = coordinator
        .store_mut()
        .collect(narrata_store::RetentionPolicy {
            now: 100,
            grace_seconds: 0,
            dry_run: false,
        })
        .unwrap();
    assert!(sweep.reachable > 0);
    assert!(coordinator.store().integrity_scan().unwrap().is_empty());
}

#[test]
fn archive_bundle_and_gc_faults_are_atomic() {
    let mut coordinator = coordinator();
    let committed = coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap();

    let before_objects = coordinator.store().list_objects().unwrap();
    let before_archives = coordinator.store().list_timeline_archives().unwrap();
    coordinator
        .store_mut()
        .inject_fault(Some(FaultPoint::ArchiveRefCas));
    assert!(
        coordinator
            .seal_complete_recording(RefName::new("sealed").unwrap(), 3)
            .is_err()
    );
    assert_eq!(coordinator.store().list_objects().unwrap(), before_objects);
    assert_eq!(
        coordinator.store().list_timeline_archives().unwrap(),
        before_archives
    );
    coordinator.store_mut().inject_fault(None);

    let bundle =
        CheckpointBundle::export(coordinator.store(), committed.commit, &BTreeSet::new()).unwrap();
    let mut imported = MemoryStore::new();
    imported.inject_fault(Some(FaultPoint::BundleImportRef));
    let target = RefKey::save(
        RefName::new("import").unwrap(),
        RefName::new("slot").unwrap(),
    );
    assert!(bundle.import(&mut imported, target, None, 4).is_err());
    assert!(imported.list_objects().unwrap().is_empty());
    assert!(imported.list_refs().unwrap().is_empty());

    for point in [FaultPoint::GcMark, FaultPoint::GcSweep] {
        let before = coordinator.store().list_objects().unwrap();
        coordinator.store_mut().inject_fault(Some(point));
        assert!(
            coordinator
                .store_mut()
                .collect(narrata_store::RetentionPolicy::default())
                .is_err()
        );
        assert_eq!(coordinator.store().list_objects().unwrap(), before);
    }
}
