#![allow(clippy::panic, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc};

use narrata_core::{
    ExecutionId, InputId, Value, builtin_host_capabilities,
    program::{encode_program_artifact, load_program},
    runtime::CheckedRuntimeInput,
};
use narrata_store::{
    BranchId, CommitTransaction, EffectClaimResult, FaultPoint, InitialRecordingMode, LeaseId,
    LedgerFence, RefKey, RefMutation, RefName, SaveStore, SessionCoordinator, StoreError,
};
use narrata_store_sqlite::SqliteStore;
use narrata_testkit::generator::{
    branch_call_choice_v0, recorded_query_v0, statechart_parallel_history_v0,
};

struct TestDb(PathBuf);

impl TestDb {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "narrata-{name}-{}-{}.sqlite3",
            std::process::id(),
            std::thread::current().name().unwrap_or("thread")
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
        Self(path)
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(self.0.with_extension("sqlite3-wal"));
        let _ = std::fs::remove_file(self.0.with_extension("sqlite3-shm"));
    }
}

fn program() -> Arc<narrata_core::CheckedProgram> {
    load_program(
        &encode_program_artifact(&branch_call_choice_v0()),
        &Default::default(),
    )
    .unwrap()
}

fn query_program() -> Arc<narrata_core::CheckedProgram> {
    load_program(
        &encode_program_artifact(&recorded_query_v0().unwrap()),
        &Default::default(),
    )
    .unwrap()
}

fn statechart_program() -> Arc<narrata_core::CheckedProgram> {
    load_program(
        &encode_program_artifact(&statechart_parallel_history_v0().unwrap()),
        &Default::default(),
    )
    .unwrap()
}

#[test]
fn statechart_invocation_and_pending_effect_restore_after_reopen() {
    let db = TestDb::new("statechart-reopen");
    let program = statechart_program();
    let mut coordinator = SessionCoordinator::create_with_capabilities(
        SqliteStore::open(&db.0).unwrap(),
        Arc::clone(&program),
        ExecutionId::from_u128(700),
        RefName::new("statechart").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Complete,
        1,
        &builtin_host_capabilities().unwrap(),
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
        SqliteStore::open(&db.0).unwrap(),
        program,
        ExecutionId::from_u128(700),
        RefName::new("statechart").unwrap(),
        BranchId::from_u128(1),
        &builtin_host_capabilities().unwrap(),
    )
    .unwrap();
    assert_eq!(reopened.state().as_ref(), expected.as_ref());
    assert!(reopened.state().pending_effect().is_some());
    assert!(reopened.store().integrity_scan().unwrap().is_empty());
}

#[test]
fn reopens_and_restores_a_committed_session() {
    let db = TestDb::new("reopen");
    let store = SqliteStore::open(&db.0).unwrap();
    assert_eq!(store.synchronous().unwrap(), 2);
    let mut coordinator = SessionCoordinator::create(
        store,
        program(),
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
        SqliteStore::open(&db.0).unwrap(),
        program(),
        ExecutionId::from_u128(1),
        RefName::new("session").unwrap(),
        BranchId::from_u128(1),
    )
    .unwrap();
    assert_eq!(reopened.timeline().cursor, committed.commit);
    assert!(reopened.store().integrity_scan().unwrap().is_empty());
}

#[test]
fn two_connections_cannot_silently_overwrite_a_slot() {
    let db = TestDb::new("cas");
    let mut coordinator = SessionCoordinator::create(
        SqliteStore::open(&db.0).unwrap(),
        program(),
        ExecutionId::from_u128(1),
        RefName::new("session").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Standard,
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

    let mut first = SqliteStore::open(&db.0).unwrap();
    let mut second = SqliteStore::open(&db.0).unwrap();
    let key = RefKey::save(
        RefName::new("player").unwrap(),
        RefName::new("slot").unwrap(),
    );
    let transaction = CommitTransaction {
        refs: vec![RefMutation {
            key: key.clone(),
            expected: None,
            next: Some(committed.commit),
        }],
        ..CommitTransaction::default()
    };
    first.commit(transaction.clone()).unwrap();
    let error = second.commit(transaction).unwrap_err();
    assert!(matches!(error, StoreError::RefConflict(_)));
    assert_eq!(
        second.read_ref(&key).unwrap().unwrap().commit,
        committed.commit
    );
}

#[test]
fn injected_commit_failure_rolls_back_sqlite_transaction() {
    let db = TestDb::new("fault");
    let mut coordinator = SessionCoordinator::create(
        SqliteStore::open(&db.0).unwrap(),
        program(),
        ExecutionId::from_u128(1),
        RefName::new("session").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Standard,
        1,
    )
    .unwrap();
    let before_objects = coordinator.store().list_objects().unwrap();
    let before_refs = coordinator.store().list_refs().unwrap();
    coordinator
        .store_mut()
        .inject_fault(Some(FaultPoint::TransactionCommit));
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
}

#[test]
fn effect_ledger_response_and_fence_survive_reopen() {
    let db = TestDb::new("effect-reopen");
    let execution = ExecutionId::from_u128(30);
    let session = RefName::new("effect-session").unwrap();
    let branch = BranchId::from_u128(1);
    let program = query_program();
    let host = builtin_host_capabilities().unwrap();
    let mut coordinator = SessionCoordinator::create_with_capabilities(
        SqliteStore::open(&db.0).unwrap(),
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
        SqliteStore::open(&db.0).unwrap(),
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
    assert_eq!(reopened.store().list_effects(execution).unwrap().len(), 1);
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
    let db = TestDb::new("effect-claim");
    let execution = ExecutionId::from_u128(31);
    let session = RefName::new("claim-session").unwrap();
    let branch = BranchId::from_u128(1);
    let program = query_program();
    let host = builtin_host_capabilities().unwrap();
    let mut creator = SessionCoordinator::create_with_capabilities(
        SqliteStore::open(&db.0).unwrap(),
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
    let mut first = SessionCoordinator::open_with_capabilities(
        SqliteStore::open(&db.0).unwrap(),
        Arc::clone(&program),
        execution,
        session.clone(),
        branch,
        &host,
    )
    .unwrap();
    let mut second = SessionCoordinator::open_with_capabilities(
        SqliteStore::open(&db.0).unwrap(),
        program,
        execution,
        session,
        branch,
        &host,
    )
    .unwrap();
    let first_lease = LeaseId::from_u128(1);
    first.claim_pending_effect(first_lease, 3, 30).unwrap();
    assert!(matches!(
        second
            .claim_pending_effect(LeaseId::from_u128(2), 4, 40)
            .unwrap(),
        EffectClaimResult::Leased { lease, expires_at: 30 } if lease == first_lease
    ));
}
