#![allow(clippy::panic, clippy::unwrap_used)]

use std::{path::PathBuf, sync::Arc};

use narrata_core::{
    ExecutionId, InputId,
    program::{encode_program_artifact, load_program},
    runtime::CheckedRuntimeInput,
};
use narrata_store::{
    BranchId, CommitTransaction, FaultPoint, InitialRecordingMode, RefKey, RefMutation, RefName,
    SaveStore, SessionCoordinator, StoreError,
};
use narrata_store_sqlite::SqliteStore;
use narrata_testkit::generator::branch_call_choice_v0;

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
