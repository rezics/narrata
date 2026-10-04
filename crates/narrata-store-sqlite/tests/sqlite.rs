#![allow(clippy::panic, clippy::unwrap_used)]

mod corpus;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use narrata_core::{
    CommitId, ExecutionId, InputId, ObjectId, Value, builtin_host_capabilities,
    codec::sha256,
    program::{encode_program_artifact, load_program},
    runtime::CheckedRuntimeInput,
};
use narrata_storage::StorageError;
use narrata_store::{
    BranchId, EffectClaimResult, InitialRecordingMode, LeaseId, LedgerFence, RefName, SaveStore,
    SessionCoordinator, StoreError, scan_all,
};
use narrata_testkit::generator::{
    branch_call_choice_v0, recorded_query_v0, statechart_parallel_history_v0,
};
use serde_json::Value as Json;
use tempfile::TempDir;

fn checked(
    artifact: &narrata_core::program::ProgramArtifactV0,
) -> Arc<narrata_core::CheckedProgram> {
    load_program(&encode_program_artifact(artifact), &Default::default()).unwrap()
}

fn database() -> (TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saves.sqlite3");
    (directory, path)
}

fn effects(store: &impl SaveStore, execution: ExecutionId) -> usize {
    scan_all(
        |after| store.scan_effects(execution, after, 10),
        |entry: &narrata_store::EffectLedgerEntry| entry.effect,
    )
    .unwrap()
    .len()
}

#[test]
fn statechart_invocation_and_pending_effect_restore_after_reopen() {
    let (_directory, path) = database();
    let program = checked(&statechart_parallel_history_v0().unwrap());
    let host = builtin_host_capabilities().unwrap();
    let mut coordinator = SessionCoordinator::create_with_capabilities(
        narrata_store_sqlite::open(&path).unwrap(),
        Arc::clone(&program),
        ExecutionId::from_u128(700),
        RefName::new("statechart").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Complete,
        1,
        &host,
    )
    .unwrap();
    let committed = coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap();
    let expected = committed.state.clone();
    drop(coordinator);

    let reopened = SessionCoordinator::open_with_capabilities(
        narrata_store_sqlite::open(&path).unwrap(),
        program,
        ExecutionId::from_u128(700),
        RefName::new("statechart").unwrap(),
        BranchId::from_u128(1),
        &host,
    )
    .unwrap();
    assert_eq!(reopened.state().as_ref(), expected.as_ref());
    assert!(reopened.state().pending_effect().is_some());
    assert!(reopened.store().integrity_scan().unwrap().is_empty());
}

#[test]
fn reopens_and_restores_a_committed_session() {
    let (_directory, path) = database();
    let program = checked(&branch_call_choice_v0());
    let mut coordinator = SessionCoordinator::create(
        narrata_store_sqlite::open(&path).unwrap(),
        Arc::clone(&program),
        ExecutionId::from_u128(1),
        RefName::new("session").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Complete,
        1,
    )
    .unwrap();
    let committed = coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap();
    drop(coordinator);

    let reopened = SessionCoordinator::open(
        narrata_store_sqlite::open(&path).unwrap(),
        program,
        ExecutionId::from_u128(1),
        RefName::new("session").unwrap(),
        BranchId::from_u128(1),
    )
    .unwrap();
    assert_eq!(reopened.timeline().cursor, committed.commit);
    assert!(reopened.store().integrity_scan().unwrap().is_empty());
}

#[test]
fn effect_ledger_response_and_fence_survive_reopen() {
    let (_directory, path) = database();
    let execution = ExecutionId::from_u128(30);
    let session = RefName::new("effect-session").unwrap();
    let branch = BranchId::from_u128(1);
    let program = checked(&recorded_query_v0().unwrap());
    let host = builtin_host_capabilities().unwrap();
    let mut coordinator = SessionCoordinator::create_with_capabilities(
        narrata_store_sqlite::open(&path).unwrap(),
        Arc::clone(&program),
        execution,
        session.clone(),
        branch,
        InitialRecordingMode::Standard,
        1,
        &host,
    )
    .unwrap();
    coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap();
    let lease = LeaseId::from_u128(1);
    coordinator.claim_pending_effect(lease, 3, 30).unwrap();
    coordinator
        .complete_pending_effect(lease, Value::I64(42), 4)
        .unwrap();
    drop(coordinator);

    let mut reopened = SessionCoordinator::open_with_capabilities(
        narrata_store_sqlite::open(&path).unwrap(),
        program,
        execution,
        session,
        branch,
        &host,
    )
    .unwrap();
    assert_eq!(
        reopened.store().current_ledger_fence(execution).unwrap(),
        LedgerFence::from_u64(1)
    );
    assert_eq!(effects(reopened.store(), execution), 1);
    assert!(
        reopened
            .recover_pending_effect(Default::default(), 5)
            .unwrap()
            .is_some()
    );
    assert!(reopened.store().integrity_scan().unwrap().is_empty());
}

#[test]
fn concurrent_sqlite_claimants_observe_the_same_lease() {
    let (_directory, path) = database();
    let execution = ExecutionId::from_u128(31);
    let session = RefName::new("claim-session").unwrap();
    let branch = BranchId::from_u128(1);
    let program = checked(&recorded_query_v0().unwrap());
    let host = builtin_host_capabilities().unwrap();
    let mut creator = SessionCoordinator::create_with_capabilities(
        narrata_store_sqlite::open(&path).unwrap(),
        Arc::clone(&program),
        execution,
        session.clone(),
        branch,
        InitialRecordingMode::Standard,
        1,
        &host,
    )
    .unwrap();
    creator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap();
    drop(creator);
    let open = || {
        SessionCoordinator::open_with_capabilities(
            narrata_store_sqlite::open(&path).unwrap(),
            Arc::clone(&program),
            execution,
            session.clone(),
            branch,
            &host,
        )
        .unwrap()
    };
    let (mut first, mut second) = (open(), open());
    let first_lease = LeaseId::from_u128(1);
    first.claim_pending_effect(first_lease, 3, 30).unwrap();
    assert!(matches!(
        second
            .claim_pending_effect(LeaseId::from_u128(2), 4, 40)
            .unwrap(),
        EffectClaimResult::Leased { lease, expires_at: 30 } if lease == first_lease
    ));
}

fn manifest(corpus: &str) -> Json {
    serde_json::from_slice(&std::fs::read(corpus::fixture(corpus).join("manifest.json")).unwrap())
        .unwrap()
}

/// Copies a frozen database into a scratch directory after checking it is the recorded one;
/// frozen corpora are never opened in place.
fn copy_frozen(corpus: &str, file: &str, manifest: &Json) -> (TempDir, PathBuf) {
    let source = corpus::fixture(corpus).join(file);
    let bytes = std::fs::read(&source).unwrap();
    assert_eq!(
        hex::encode(sha256(&bytes)),
        manifest["artifacts"][file]["sha256"].as_str().unwrap(),
        "{} differs from its manifest",
        source.display()
    );
    let directory = tempfile::tempdir().unwrap();
    let copy = directory.path().join(file);
    std::fs::write(&copy, bytes).unwrap();
    (directory, copy)
}

/// The bytes of an identity written as `prefix:hex`.
fn bytes<const N: usize>(value: &Json) -> [u8; N] {
    let (_, hex) = value.as_str().unwrap().split_once(':').unwrap();
    hex::decode(hex).unwrap().try_into().unwrap()
}

fn commit(value: &Json) -> CommitId {
    CommitId::from_bytes(bytes(value))
}

/// Opens the corpus session, checks the recorded Effect is still recorded after a rewind, and
/// commits one more turn.
fn continue_session(store: narrata_store_sqlite::SqliteStore, session: &Json) {
    let program = checked(&recorded_query_v0().unwrap());
    let execution = ExecutionId::from_bytes(bytes(&session["execution"]));
    let mut coordinator = SessionCoordinator::open_with_capabilities(
        store,
        program,
        execution,
        RefName::new(session["name"].as_str().unwrap()).unwrap(),
        session["selected_branch"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
        &builtin_host_capabilities().unwrap(),
    )
    .unwrap();
    assert_eq!(coordinator.timeline().cursor, commit(&session["cursor"]));
    let pending = coordinator.state().pending().unwrap().interaction_id();
    let next = coordinator
        .dispatch(
            CheckedRuntimeInput::advance(InputId::from_u128(2), pending),
            Default::default(),
            100,
        )
        .unwrap();
    assert!(!next.reused);
    assert!(coordinator.state().pending().is_none());

    coordinator
        .rewind_to(commit(&session["pending_effect_commit"]), 101)
        .unwrap();
    assert!(matches!(
        coordinator
            .claim_pending_effect(LeaseId::from_u128(9), 102, 130)
            .unwrap(),
        EffectClaimResult::Recorded(_)
    ));
    assert!(
        coordinator
            .recover_pending_effect(Default::default(), 103)
            .unwrap()
            .unwrap()
            .reused
    );
    assert!(coordinator.store().integrity_scan().unwrap().is_empty());
}

/// The recorded roots and ledger without revisions, which migration reassigns, and without
/// objects, which are compared on their own.
fn without_revisions(mut contents: Json) -> Json {
    contents.as_object_mut().unwrap().remove("objects");
    for root in ["refs", "catalogs", "archives", "compound_saves"] {
        for entry in contents[root].as_array_mut().unwrap() {
            entry.as_object_mut().unwrap().remove("revision");
        }
    }
    contents
}

#[test]
fn schema_v2_store_is_refused_then_migrated_and_continues() {
    let manifest = manifest("store-sqlite-v2");
    let (directory, source) = copy_frozen("store-sqlite-v2", "save-v2.sqlite3", &manifest);
    let original = std::fs::read(&source).unwrap();

    let refused = narrata_store_sqlite::open(&source).unwrap_err();
    assert!(
        matches!(&refused, StoreError::Storage(StorageError::Format(message)) if message.contains("migrate-v2")),
        "{refused:?}"
    );
    assert_eq!(std::fs::read(&source).unwrap(), original);

    let target = directory.path().join("migrated.sqlite3");
    let store = narrata_store_sqlite::migrate_v2(&source, &target).unwrap();
    assert_eq!(
        std::fs::read(&source).unwrap(),
        original,
        "the source is untouched"
    );
    assert!(matches!(
        narrata_store_sqlite::migrate_v2(&source, &target),
        Err(StoreError::Storage(StorageError::Invalid(_)))
    ));

    let session = &manifest["session"];
    let execution = ExecutionId::from_bytes(bytes(&session["execution"]));
    let contents = &manifest["contents"];
    assert_eq!(
        corpus::contents(&store, execution, &[InputId::from_u128(1)]),
        without_revisions(contents.clone())
    );
    let objects = contents["objects"].as_array().unwrap();
    let ids = objects
        .iter()
        .map(|object| ObjectId::from_bytes(bytes(&object["id"])))
        .collect::<Vec<_>>();
    for (object, stored) in objects.iter().zip(store.get_objects(&ids).unwrap()) {
        let stored = stored.unwrap();
        assert_eq!(object["kind"], stored.kind().code());
        assert_eq!(object["schema"], stored.schema());
        assert_eq!(object["sha256"], hex::encode(sha256(stored.bytes())));
    }
    let image = corpus::layout_image(&store);
    assert_eq!(image["objects"].as_array().unwrap().len(), objects.len());
    // Each object keeps its insertion time as the time GC measures its grace period from.
    let touched = image["keys"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|key| key["space"] == "touch")
        .map(|key| {
            (
                key["key"].as_str().unwrap().to_owned(),
                key["value"].clone(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    for object in objects {
        let inserted_at = object["inserted_at"].as_u64().unwrap();
        assert!(inserted_at < 24, "the corpus records small times");
        let key = object["id"].as_str().unwrap().trim_start_matches("object:");
        // `{0: inserted_at}` in canonical CBOR.
        assert_eq!(touched[key], hex::encode([0xa1, 0x00, inserted_at as u8]));
    }
    assert!(store.integrity_scan().unwrap().is_empty());
    drop(store);

    continue_session(narrata_store_sqlite::open(&target).unwrap(), session);
}

#[test]
fn failed_migration_leaves_no_target() {
    let (directory, path) = database();
    std::fs::write(&path, b"not a database").unwrap();
    let target = directory.path().join("target.sqlite3");
    assert!(narrata_store_sqlite::migrate_v2(&path, &target).is_err());
    assert!(!target.exists());
    assert!(!Path::new(&format!("{}-wal", target.display())).exists());
}

#[test]
fn frozen_layout_v1_store_opens_unchanged_and_continues() {
    let manifest = manifest("store-layout-v1");
    let (_directory, path) = copy_frozen("store-layout-v1", "saves.sqlite3", &manifest);
    let store = narrata_store_sqlite::open(&path).unwrap();
    assert_eq!(corpus::layout_image(&store), manifest["image"]);
    assert!(store.integrity_scan().unwrap().is_empty());
    continue_session(store, &manifest["session"]);
}
