#![allow(clippy::panic, clippy::unwrap_used)]

use std::time::Duration;

use narrata_storage::{Batch, Expect, KeySpace, StorageBackend, StorageError};
use narrata_storage_sqlite::{APPLICATION_ID, SCHEMA_VERSION, SqliteBackend, SqliteOptions};
use rusqlite::Connection;

const SPACE: KeySpace = KeySpace::new(0);

fn format_error(result: Result<SqliteBackend, StorageError>) -> String {
    match result {
        Err(StorageError::Format(message)) => message,
        other => panic!("expected a format error, got {other:?}"),
    }
}

#[test]
fn foreign_databases_are_refused_and_left_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("foreign.sqlite");
    {
        // Shaped like a narrata-store-sqlite save database: tables, no application id.
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
                 INSERT INTO schema_meta VALUES ('schema_version', 2);",
            )
            .unwrap();
    }
    let before = std::fs::read(&path).unwrap();
    format_error(SqliteBackend::open(&path));
    assert_eq!(std::fs::read(&path).unwrap(), before);

    let text = dir.path().join("text.sqlite");
    std::fs::write(
        &text,
        b"definitely not a database, but long enough to have a header....",
    )
    .unwrap();
    format_error(SqliteBackend::open(&text));
}

#[test]
fn newer_schema_versions_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite");
    drop(SqliteBackend::open(&path).unwrap());
    {
        let connection = Connection::open(&path).unwrap();
        let application_id: i32 = connection
            .pragma_query_value(None, "application_id", |row| row.get(0))
            .unwrap();
        assert_eq!(application_id, APPLICATION_ID);
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
    }
    let message = format_error(SqliteBackend::open(&path));
    assert!(message.contains("newer"), "{message}");
}

#[test]
fn another_writer_holding_the_lock_makes_a_batch_busy_without_effect() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite");
    let mut backend = SqliteBackend::open_with(
        &path,
        SqliteOptions {
            busy_timeout: Duration::from_millis(50),
            ..SqliteOptions::default()
        },
    )
    .unwrap();
    let mut other = Connection::open(&path).unwrap();
    let lock = other
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();

    let batch = Batch::new().put(SPACE, b"k", b"v", Expect::Absent);
    let error = backend.apply(&batch).unwrap_err();
    assert_eq!(error, StorageError::Busy);
    assert!(error.retryable() && !error.outcome_unknown());
    assert_eq!(backend.read_key(SPACE, b"k").unwrap(), None);

    lock.rollback().unwrap();
    backend.apply(&batch).unwrap();
    assert!(backend.read_key(SPACE, b"k").unwrap().is_some());
}

#[test]
fn concurrent_openers_of_a_new_file_share_one_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite");
    let opened = std::thread::scope(|scope| {
        let openers = (0..4)
            .map(|_| scope.spawn(|| SqliteBackend::open(&path)))
            .collect::<Vec<_>>();
        openers
            .into_iter()
            .map(|opener| opener.join().unwrap())
            .collect::<Vec<_>>()
    });
    let mut handles = opened.into_iter().map(Result::unwrap).collect::<Vec<_>>();
    let first = handles[0]
        .apply(&Batch::new().put(SPACE, b"k", b"v", Expect::Absent))
        .unwrap();
    for handle in &handles {
        assert_eq!(
            handle
                .read_key(SPACE, b"k")
                .unwrap()
                .map(|value| value.revision),
            first.revision
        );
    }
}
