#![allow(clippy::panic, clippy::unwrap_used)]

use std::{
    sync::{Barrier, mpsc},
    time::{Duration, Instant},
};

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
    for round in 0..24 {
        let path = dir.path().join(format!("store-{round}.sqlite"));
        let start = Barrier::new(8);
        let mut handles = std::thread::scope(|scope| {
            let openers = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        start.wait();
                        SqliteBackend::open(&path).unwrap()
                    })
                })
                .collect::<Vec<_>>();
            openers
                .into_iter()
                .map(|opener| opener.join().unwrap())
                .collect::<Vec<_>>()
        });
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
}

#[test]
fn concurrent_openers_and_writers_preserve_every_batch() {
    let dir = tempfile::tempdir().unwrap();
    for round in 0..24 {
        let path = dir.path().join(format!("store-{round}.sqlite"));
        let start = Barrier::new(8);
        let handles = std::thread::scope(|scope| {
            let writers = (0..8_u8)
                .map(|writer| {
                    let path = &path;
                    let start = &start;
                    scope.spawn(move || {
                        start.wait();
                        let mut backend = SqliteBackend::open(path).unwrap();
                        for write in 0..4_u8 {
                            let key = [writer, write];
                            backend
                                .apply(&Batch::new().put(SPACE, key, key, Expect::Absent))
                                .unwrap();
                        }
                        backend
                    })
                })
                .collect::<Vec<_>>();
            writers
                .into_iter()
                .map(|writer| writer.join().unwrap())
                .collect::<Vec<_>>()
        });
        let page = handles[0].scan_keys(SPACE, b"", None, 32).unwrap();
        assert!(!page.more);
        assert_eq!(page.entries.len(), 32);
        let mut revisions = Vec::new();
        for entry in page.entries {
            assert_eq!(entry.value, entry.key);
            revisions.push(entry.revision);
        }
        revisions.sort_unstable();
        revisions.dedup();
        assert_eq!(revisions.len(), 32);
    }
}

// A read transaction in rollback mode permits the format-check transaction but prevents
// the WAL switch from acquiring an exclusive lock.
fn rollback_mode_reader(path: &std::path::Path) -> Connection {
    drop(SqliteBackend::open(path).unwrap());
    let reader = Connection::open(path).unwrap();
    let mode: String = reader
        .query_row("PRAGMA journal_mode = DELETE", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "delete");
    reader.execute_batch("BEGIN; SELECT * FROM meta;").unwrap();
    reader
}

#[test]
fn opening_waits_for_a_reader_blocking_the_wal_switch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite");
    let reader = rollback_mode_reader(&path);
    let (send, receive) = mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let result = SqliteBackend::open_with(
                &path,
                SqliteOptions {
                    busy_timeout: Duration::from_secs(2),
                    ..SqliteOptions::default()
                },
            );
            send.send(result).unwrap();
        });
        assert!(matches!(
            receive.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        reader.execute_batch("ROLLBACK").unwrap();
        let mut backend = receive
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        backend
            .apply(&Batch::new().put(SPACE, b"k", b"v", Expect::Absent))
            .unwrap();
    });
}

#[test]
fn opening_reports_busy_when_the_wal_switch_exhausts_its_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite");
    let reader = rollback_mode_reader(&path);
    for timeout in [Duration::from_millis(80), Duration::ZERO] {
        let started = Instant::now();
        let error = SqliteBackend::open_with(
            &path,
            SqliteOptions {
                busy_timeout: timeout,
                ..SqliteOptions::default()
            },
        )
        .unwrap_err();
        let elapsed = started.elapsed();
        assert_eq!(error, StorageError::Busy);
        assert!(error.retryable() && !error.outcome_unknown());
        assert!(elapsed >= timeout, "returned Busy after {elapsed:?}");
        assert!(elapsed < Duration::from_secs(2), "waited {elapsed:?}");
    }
    reader.execute_batch("ROLLBACK").unwrap();
    SqliteBackend::open(&path).unwrap();
}
