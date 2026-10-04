#![allow(clippy::panic, clippy::unwrap_used)]

#[macro_use]
mod support;

use std::{collections::BTreeSet, sync::Arc};

use narrata_core::{
    ExecutionId, InputId,
    program::{encode_program_artifact, load_program},
    runtime::{CheckedRuntimeInput, PendingInteractionV0},
};
use narrata_store::{
    BranchId, CheckpointBundle, InitialRecordingMode, RefKey, RefName, RefRevision, SaveResult,
    SaveStore, SessionCoordinator, Store, TimelineArchiveBundle, TimelineImportMapping,
    TimelineOperationId,
};
use narrata_testkit::generator::branch_call_choice_v0;
use support::{Backend, image};

fn program() -> Arc<narrata_core::CheckedProgram> {
    load_program(
        &encode_program_artifact(&branch_call_choice_v0()),
        &Default::default(),
    )
    .unwrap()
}

fn coordinator<B: Backend>(recording: InitialRecordingMode) -> SessionCoordinator<Store<B::Inner>> {
    SessionCoordinator::create(
        B::store(),
        program(),
        ExecutionId::from_u128(1),
        RefName::new("session").unwrap(),
        BranchId::from_u128(1),
        recording,
        1,
    )
    .unwrap()
}

fn commit_rewind_replay_and_fork_preserve_both_futures<B: Backend>() {
    let mut coordinator = coordinator::<B>(InitialRecordingMode::Standard);
    let genesis = coordinator.timeline().cursor;
    let first = coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap();
    assert!(!first.reused);

    coordinator.rewind_to(genesis, 3).unwrap();
    let replay = coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            4,
        )
        .unwrap();
    assert!(replay.reused);
    assert_eq!(replay.commit, first.commit);

    let pending = coordinator.state().pending().unwrap();
    let interaction = pending.interaction_id();
    let second = coordinator
        .dispatch(
            CheckedRuntimeInput::advance(InputId::from_u128(2), interaction),
            Default::default(),
            5,
        )
        .unwrap();
    let fork_point = first.commit;
    coordinator.rewind_to(fork_point, 6).unwrap();
    let PendingInteractionV0::Say { interaction_id, .. } = coordinator.state().pending().unwrap()
    else {
        panic!("expected say");
    };
    let forked = coordinator
        .dispatch(
            CheckedRuntimeInput::advance(InputId::from_u128(3), *interaction_id),
            Default::default(),
            7,
        )
        .unwrap();
    assert_ne!(forked.branch, second.branch);
    assert!(
        coordinator
            .store()
            .get_object(narrata_core::ObjectId::from_bytes(
                *second.commit.as_bytes()
            ))
            .unwrap()
            .is_some()
    );
}

fn save_uses_cas_and_checkpoint_round_trips<B: Backend>() {
    let mut coordinator = coordinator::<B>(InitialRecordingMode::Standard);
    let result = coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap();
    let owner = RefName::new("player").unwrap();
    let slot = RefName::new("slot-1").unwrap();
    assert!(matches!(
        coordinator
            .save(
                owner.clone(),
                slot.clone(),
                None,
                TimelineOperationId::from_u128(1),
                3,
            )
            .unwrap(),
        SaveResult::Saved(_)
    ));
    assert!(
        coordinator
            .save(
                owner.clone(),
                slot.clone(),
                None,
                TimelineOperationId::from_u128(2),
                4,
            )
            .is_err()
    );

    let bundle =
        CheckpointBundle::export(coordinator.store(), result.commit, &BTreeSet::new()).unwrap();
    let encoded = bundle.to_bytes().unwrap();
    let decoded = CheckpointBundle::from_bytes(&encoded, Default::default()).unwrap();
    let mut imported = B::store();
    let target = RefKey::save(
        RefName::new("other").unwrap(),
        RefName::new("slot").unwrap(),
    );
    decoded
        .import(&mut imported, target.clone(), None, 5)
        .unwrap();
    assert_eq!(
        imported.read_ref(&target).unwrap().unwrap().commit,
        result.commit
    );
}

fn complete_timeline_archive_round_trips_catalog_and_session<B: Backend>() {
    let mut coordinator = coordinator::<B>(InitialRecordingMode::Complete);
    let first = coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap();
    coordinator
        .save(
            RefName::new("player").unwrap(),
            RefName::new("slot").unwrap(),
            None,
            TimelineOperationId::from_u128(20),
            3,
        )
        .unwrap();
    coordinator
        .bookmark(
            RefName::new("player").unwrap(),
            RefName::new("mark").unwrap(),
            TimelineOperationId::from_u128(21),
            4,
        )
        .unwrap();
    let archive = TimelineArchiveBundle::export(
        coordinator.store(),
        ExecutionId::from_u128(1),
        Some(coordinator.timeline()),
        &BTreeSet::new(),
    )
    .unwrap();
    assert_eq!(archive.manifest.active.unwrap().cursor, first.commit);
    let bytes = archive.to_bytes().unwrap();
    let archive = TimelineArchiveBundle::from_bytes(&bytes, Default::default()).unwrap();
    let branch = archive.manifest.branch_heads[0].branch;
    let mapping = TimelineImportMapping {
        archive_name: RefName::new("archive").unwrap(),
        save_owner: RefName::new("imported").unwrap(),
        bookmark_owner: RefName::new("imported").unwrap(),
        branch_ids: [(branch, BranchId::from_u128(99))].into_iter().collect(),
        save_names: [(
            RefName::new("slot").unwrap(),
            RefName::new("slot-2").unwrap(),
        )]
        .into_iter()
        .collect(),
        bookmark_names: [(
            RefName::new("mark").unwrap(),
            RefName::new("mark-2").unwrap(),
        )]
        .into_iter()
        .collect(),
        session_name: Some(RefName::new("imported-session").unwrap()),
    };
    let mut imported = B::store();
    archive.import(&mut imported, mapping, 5).unwrap();
    assert!(
        imported
            .read_catalog_head(&narrata_store::CatalogRefKey::new(ExecutionId::from_u128(
                1
            )))
            .unwrap()
            .is_some()
    );
}

fn complete_save_delete_is_a_tombstone_until_catalog_root_is_removed<B: Backend>() {
    let mut coordinator = coordinator::<B>(InitialRecordingMode::Complete);
    coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap();
    let owner = RefName::new("player").unwrap();
    let slot = RefName::new("slot").unwrap();
    let SaveResult::Saved(value) = coordinator
        .save(
            owner.clone(),
            slot.clone(),
            None,
            TimelineOperationId::from_u128(10),
            3,
        )
        .unwrap()
    else {
        panic!("save deferred");
    };
    coordinator
        .delete_save(
            owner,
            slot,
            value.revision,
            TimelineOperationId::from_u128(11),
            4,
        )
        .unwrap();
    let before = image(coordinator.store().backend());
    let report = coordinator
        .store_mut()
        .collect(narrata_store::RetentionPolicy {
            now: u64::MAX,
            grace_seconds: 0,
            dry_run: false,
        })
        .unwrap();
    assert_eq!(image(coordinator.store().backend()).objects, before.objects);
    assert!(report.removed.values().all(|value| value.objects == 0));
}

fn revision_is_opaque_and_nonzero<B: Backend>() {
    assert_eq!(RefRevision::from_u64(0), None);
    let mut coordinator = coordinator::<B>(InitialRecordingMode::Standard);
    let active = RefKey::active(RefName::new("session").unwrap()).unwrap();
    let revision = |coordinator: &SessionCoordinator<Store<B::Inner>>| {
        coordinator
            .store()
            .read_ref(&active)
            .unwrap()
            .unwrap()
            .revision
    };
    let created = revision(&coordinator);
    assert_ne!(created.get(), 0);
    assert_eq!(RefRevision::from_u64(created.get()), Some(created));
    coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap();
    assert_ne!(revision(&coordinator), created);
}

backend_tests!(
    commit_rewind_replay_and_fork_preserve_both_futures,
    save_uses_cas_and_checkpoint_round_trips,
    complete_timeline_archive_round_trips_catalog_and_session,
    complete_save_delete_is_a_tombstone_until_catalog_root_is_removed,
    revision_is_opaque_and_nonzero,
);
