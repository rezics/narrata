//! Row-level SQLite backend for the Narrata storage contract (ADR 0012).
//!
//! Every call touches only the rows it needs: reads are primary-key lookups or index range
//! scans, and each batch is one `BEGIN IMMEDIATE` transaction. Connections use WAL with
//! `synchronous=FULL`, so several connections, including other processes, may share a file.

#![forbid(unsafe_code)]

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use narrata_storage::{
    Applied, Batch, Capabilities, Conflict, Durability, KeyAction, KeyEntry, KeyPage, KeySpace,
    KeyValue, Limit, Limits, ObjectDigest, ObjectPage, Revision, StorageBackend, StorageError,
    WriteModel,
};
use rusqlite::{
    Connection, Error as SqlError, ErrorCode, OptionalExtension, Params, TransactionBehavior,
    params,
};

/// `PRAGMA application_id` of narrata-storage files: "NRST".
pub const APPLICATION_ID: i32 = 0x4E52_5354;
/// `PRAGMA user_version` this backend reads and writes.
pub const SCHEMA_VERSION: i32 = 1;

/// Objects may be large, so they keep a rowid table; keys are mostly small and read by range,
/// so they cluster by `(space, key)`. SQLite compares BLOBs bytewise, shorter prefix first,
/// which is the contract's key order.
const SCHEMA: &str = "
CREATE TABLE meta (
  name TEXT PRIMARY KEY NOT NULL,
  value INTEGER NOT NULL
) STRICT, WITHOUT ROWID;
CREATE TABLE objects (
  digest BLOB PRIMARY KEY NOT NULL CHECK (length(digest) = 32),
  bytes BLOB NOT NULL
) STRICT;
CREATE TABLE keys (
  space INTEGER NOT NULL CHECK (space BETWEEN 0 AND 65535),
  key BLOB NOT NULL,
  value BLOB NOT NULL,
  revision INTEGER NOT NULL CHECK (revision > 0),
  PRIMARY KEY (space, key)
) STRICT, WITHOUT ROWID;
INSERT INTO meta (name, value) VALUES ('last_revision', 0);
";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SqliteOptions {
    pub limits: Limits,
    /// How long a call waits for another connection's write before reporting `Busy`.
    pub busy_timeout: Duration,
}

impl Default for SqliteOptions {
    fn default() -> Self {
        Self {
            limits: Limits::DEFAULT,
            busy_timeout: Duration::from_secs(5),
        }
    }
}

/// One connection to a narrata-storage SQLite file.
#[derive(Debug)]
pub struct SqliteBackend {
    connection: Connection,
    path: PathBuf,
    limits: Limits,
}

impl SqliteBackend {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        Self::open_with(path, SqliteOptions::default())
    }

    /// Opens or creates the store at `path`. A file in another format, including a newer
    /// schema version, is refused with [`StorageError::Format`] and left untouched.
    pub fn open_with(path: impl AsRef<Path>, options: SqliteOptions) -> Result<Self, StorageError> {
        let path = path.as_ref().to_path_buf();
        let mut connection = Connection::open(&path).map_err(map_error)?;
        connection
            .busy_timeout(options.busy_timeout)
            .map_err(map_error)?;
        prepare_schema(&mut connection)?;
        // Set only after the format check, so that a foreign file is never modified. WAL lets
        // readers proceed during a write; filesystems without WAL support keep their journal
        // mode, which is slower but equally atomic and durable.
        connection
            .query_row("PRAGMA journal_mode = WAL", [], |row| {
                row.get::<_, String>(0)
            })
            .map_err(map_error)?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(map_error)?;
        Ok(Self {
            connection,
            path,
            limits: options.limits,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Creates the schema in an empty file or checks an existing one. Runs under `BEGIN
/// IMMEDIATE`, so two processes opening the same new file cannot both create it.
fn prepare_schema(connection: &mut Connection) -> Result<(), StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(map_open_error)?;
    let pragma = |name: &str| {
        transaction
            .pragma_query_value(None, name, |row| row.get::<_, i32>(0))
            .map_err(map_open_error)
    };
    match (pragma("application_id")?, pragma("user_version")?) {
        (APPLICATION_ID, SCHEMA_VERSION) => {}
        (APPLICATION_ID, version) if version > SCHEMA_VERSION => {
            return Err(StorageError::Format(format!(
                "schema version {version} is newer than the supported {SCHEMA_VERSION}"
            )));
        }
        (0, 0) if is_empty(&transaction)? => {
            transaction.execute_batch(SCHEMA).map_err(map_error)?;
            transaction
                .pragma_update(None, "application_id", APPLICATION_ID)
                .map_err(map_error)?;
            transaction
                .pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(map_error)?;
        }
        _ => {
            return Err(StorageError::Format(
                "not a narrata-storage database".to_owned(),
            ));
        }
    }
    transaction.commit().map_err(map_error)
}

fn is_empty(connection: &Connection) -> Result<bool, StorageError> {
    connection
        .query_row("SELECT count(*) FROM sqlite_schema", [], |row| {
            row.get::<_, i64>(0)
        })
        .map(|count| count == 0)
        .map_err(map_error)
}

impl StorageBackend for SqliteBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            write_model: WriteModel::AtomicConcurrent,
            durability: Durability::Durable,
            limits: self.limits,
        }
    }

    fn get_objects(
        &self,
        digests: &[ObjectDigest],
    ) -> Result<Vec<Option<Arc<[u8]>>>, StorageError> {
        self.limits.check(Limit::ReadItems, digests.len() as u64)?;
        let mut statement = self
            .connection
            .prepare_cached("SELECT bytes FROM objects WHERE digest = ?1")
            .map_err(map_error)?;
        digests
            .iter()
            .map(|digest| {
                statement
                    .query_row([digest.as_bytes().as_slice()], |row| {
                        Ok(Arc::from(row.get_ref(0)?.as_blob()?))
                    })
                    .optional()
                    .map_err(map_error)
            })
            .collect()
    }

    fn scan_objects(
        &self,
        after: Option<&ObjectDigest>,
        limit: u32,
    ) -> Result<ObjectPage, StorageError> {
        self.limits.check_page(limit)?;
        let fetch = i64::from(limit) + 1;
        let mut digests = match after {
            Some(after) => self.query(
                "SELECT digest FROM objects WHERE digest > ?1 ORDER BY digest LIMIT ?2",
                params![after.as_bytes().as_slice(), fetch],
                |row| row.get::<_, [u8; 32]>(0).map(ObjectDigest::from_bytes),
            ),
            None => self.query(
                "SELECT digest FROM objects ORDER BY digest LIMIT ?1",
                params![fetch],
                |row| row.get::<_, [u8; 32]>(0).map(ObjectDigest::from_bytes),
            ),
        }?;
        let more = digests.len() > limit as usize;
        digests.truncate(limit as usize);
        Ok(ObjectPage { digests, more })
    }

    fn read_key(&self, space: KeySpace, key: &[u8]) -> Result<Option<KeyValue>, StorageError> {
        self.limits.check(Limit::KeyBytes, key.len() as u64)?;
        read_key(&self.connection, space, key)
    }

    fn scan_keys(
        &self,
        space: KeySpace,
        prefix: &[u8],
        after: Option<&[u8]>,
        limit: u32,
    ) -> Result<KeyPage, StorageError> {
        self.limits.check_key_scan(prefix, limit)?;
        // The cursor becomes the lower bound and the prefix an upper bound, so the query is
        // an index range of at most `limit + 1` rows however deep the cursor is.
        let (lower, after_cursor) = match after {
            Some(after) if after >= prefix => (after, true),
            _ => (prefix, false),
        };
        let space = i64::from(space.get());
        let fetch = i64::from(limit) + 1;
        let entry = |row: &rusqlite::Row<'_>| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)?,
            ))
        };
        let rows = match (after_cursor, prefix_successor(prefix)) {
            (false, None) => self.query(
                "SELECT key, value, revision FROM keys
                 WHERE space = ?1 AND key >= ?2 ORDER BY key LIMIT ?3",
                params![space, lower, fetch],
                entry,
            ),
            (true, None) => self.query(
                "SELECT key, value, revision FROM keys
                 WHERE space = ?1 AND key > ?2 ORDER BY key LIMIT ?3",
                params![space, lower, fetch],
                entry,
            ),
            (false, Some(upper)) => self.query(
                "SELECT key, value, revision FROM keys
                 WHERE space = ?1 AND key >= ?2 AND key < ?3 ORDER BY key LIMIT ?4",
                params![space, lower, upper, fetch],
                entry,
            ),
            (true, Some(upper)) => self.query(
                "SELECT key, value, revision FROM keys
                 WHERE space = ?1 AND key > ?2 AND key < ?3 ORDER BY key LIMIT ?4",
                params![space, lower, upper, fetch],
                entry,
            ),
        }?;
        let more = rows.len() > limit as usize;
        let entries = rows
            .into_iter()
            .take(limit as usize)
            .map(|(key, value, revision)| {
                Ok(KeyEntry {
                    key,
                    value,
                    revision: revision_from_sql(revision)?,
                })
            })
            .collect::<Result<_, StorageError>>()?;
        Ok(KeyPage { entries, more })
    }

    fn apply(&mut self, batch: &Batch) -> Result<Applied, StorageError> {
        batch.validate(&self.capabilities())?;
        // Every early return drops the transaction, which rolls it back.
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_error)?;
        for (index, op) in batch.keys.iter().enumerate() {
            let actual = read_key(&transaction, op.space, &op.key)?;
            if !op.expect.holds(actual.as_ref().map(|value| value.revision)) {
                return Err(Conflict {
                    index,
                    space: op.space,
                    key: op.key.clone(),
                    expected: op.expect,
                    actual,
                }
                .into());
            }
        }

        let mut applied = Applied::default();
        if batch.puts_keys() {
            applied.revision = Some(allocate_revision(&transaction)?);
        }
        {
            let mut insert = transaction
                .prepare_cached(
                    "INSERT INTO objects (digest, bytes) VALUES (?1, ?2)
                     ON CONFLICT (digest) DO NOTHING",
                )
                .map_err(map_error)?;
            for (digest, bytes) in &batch.put_objects {
                let inserted = insert
                    .execute(params![digest.as_bytes().as_slice(), &bytes[..]])
                    .map_err(map_error)?;
                applied.objects_inserted += inserted as u64;
            }
            let mut delete = transaction
                .prepare_cached("DELETE FROM objects WHERE digest = ?1")
                .map_err(map_error)?;
            for digest in &batch.delete_objects {
                let deleted = delete
                    .execute([digest.as_bytes().as_slice()])
                    .map_err(map_error)?;
                applied.objects_deleted += deleted as u64;
            }

            let mut put = transaction
                .prepare_cached(
                    "INSERT INTO keys (space, key, value, revision) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT (space, key) DO UPDATE
                     SET value = excluded.value, revision = excluded.revision",
                )
                .map_err(map_error)?;
            let mut remove = transaction
                .prepare_cached("DELETE FROM keys WHERE space = ?1 AND key = ?2")
                .map_err(map_error)?;
            for op in &batch.keys {
                let space = i64::from(op.space.get());
                match (&op.action, applied.revision) {
                    (KeyAction::Put(value), Some(revision)) => {
                        put.execute(params![space, op.key, value, revision_to_sql(revision)?])
                            .map_err(map_error)?;
                    }
                    (KeyAction::Delete, _) => {
                        remove.execute(params![space, op.key]).map_err(map_error)?;
                    }
                    // `applied.revision` is set exactly when the batch puts a key.
                    (KeyAction::Put(_), None) | (KeyAction::Check, _) => {}
                }
            }
        }
        // A failed COMMIT rolls back on drop; `map_error` reports I/O failures here as
        // outcome-unknown, Busy and Full as no effect.
        transaction.commit().map_err(map_error)?;
        Ok(applied)
    }
}

impl SqliteBackend {
    fn query<T, P: Params>(
        &self,
        sql: &str,
        params: P,
        row: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>, StorageError> {
        let mut statement = self.connection.prepare_cached(sql).map_err(map_error)?;
        let rows = statement.query_map(params, row).map_err(map_error)?;
        rows.map(|row| row.map_err(map_error)).collect()
    }
}

fn read_key(
    connection: &Connection,
    space: KeySpace,
    key: &[u8],
) -> Result<Option<KeyValue>, StorageError> {
    connection
        .prepare_cached("SELECT value, revision FROM keys WHERE space = ?1 AND key = ?2")
        .map_err(map_error)?
        .query_row(params![i64::from(space.get()), key], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?))
        })
        .optional()
        .map_err(map_error)?
        .map(|(value, revision)| {
            Ok(KeyValue {
                value,
                revision: revision_from_sql(revision)?,
            })
        })
        .transpose()
}

/// Issues the next store-wide revision. The counter has its own row so that deleting the key
/// with the newest revision never lets that revision be issued again.
fn allocate_revision(connection: &Connection) -> Result<Revision, StorageError> {
    let last = connection
        .prepare_cached("SELECT value FROM meta WHERE name = 'last_revision'")
        .map_err(map_error)?
        .query_row([], |row| row.get::<_, i64>(0))
        .map_err(map_error)?;
    let next = last.checked_add(1).ok_or(StorageError::Full)?;
    connection
        .prepare_cached("UPDATE meta SET value = ?1 WHERE name = 'last_revision'")
        .map_err(map_error)?
        .execute([next])
        .map_err(map_error)?;
    revision_from_sql(next)
}

fn revision_from_sql(value: i64) -> Result<Revision, StorageError> {
    u64::try_from(value)
        .ok()
        .and_then(Revision::new)
        .ok_or_else(|| StorageError::Corrupt(format!("invalid revision {value}")))
}

fn revision_to_sql(revision: Revision) -> Result<i64, StorageError> {
    i64::try_from(revision.get()).map_err(|_| StorageError::Full)
}

/// Smallest key above every key that starts with `prefix`, if one exists.
fn prefix_successor(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut successor = prefix.to_vec();
    while let Some(last) = successor.pop() {
        if let Some(next) = last.checked_add(1) {
            successor.push(next);
            return Some(successor);
        }
    }
    None
}

fn map_open_error(error: SqlError) -> StorageError {
    match error.sqlite_error_code() {
        Some(ErrorCode::NotADatabase) => StorageError::Format("not a SQLite database".to_owned()),
        _ => map_error(error),
    }
}

fn map_error(error: SqlError) -> StorageError {
    match &error {
        SqlError::SqliteFailure(failure, _) => match failure.code {
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => StorageError::Busy,
            ErrorCode::DiskFull => StorageError::Full,
            ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase => {
                StorageError::Corrupt(error.to_string())
            }
            _ => StorageError::Io(error.to_string()),
        },
        // STRICT tables enforce column types, so a value that does not convert means damage.
        SqlError::InvalidColumnType(..)
        | SqlError::FromSqlConversionFailure(..)
        | SqlError::IntegralValueOutOfRange(..) => StorageError::Corrupt(error.to_string()),
        _ => StorageError::Io(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn prefix_successor_skips_trailing_ff() {
        assert_eq!(prefix_successor(b""), None);
        assert_eq!(prefix_successor(&[0xFF, 0xFF]), None);
        assert_eq!(prefix_successor(&[0, 0xFF]), Some(vec![1]));
        assert_eq!(prefix_successor(&[1, 2]), Some(vec![1, 3]));
    }

    #[test]
    fn connections_use_wal_and_full_sync() {
        let dir = tempfile::tempdir().unwrap();
        let backend = SqliteBackend::open(dir.path().join("store.sqlite")).unwrap();
        let pragma = |name: &str| {
            backend
                .connection
                .pragma_query_value(None, name, |row| row.get::<_, rusqlite::types::Value>(0))
                .unwrap()
        };
        assert_eq!(
            pragma("journal_mode"),
            rusqlite::types::Value::Text("wal".to_owned())
        );
        assert_eq!(pragma("synchronous"), rusqlite::types::Value::Integer(2));
        assert_eq!(
            pragma("application_id"),
            rusqlite::types::Value::Integer(i64::from(APPLICATION_ID))
        );
    }

    #[test]
    fn full_disk_rejects_the_batch_without_effect() {
        let dir = tempfile::tempdir().unwrap();
        let mut backend = SqliteBackend::open(dir.path().join("store.sqlite")).unwrap();
        let pages: i64 = backend
            .connection
            .pragma_query_value(None, "page_count", |row| row.get(0))
            .unwrap();
        backend
            .connection
            .pragma_update(None, "max_page_count", pages + 1)
            .unwrap();
        let digest = ObjectDigest::from_bytes([1; 32]);
        let batch = Batch::new().put_object(digest, vec![0; 1 << 20]).put(
            KeySpace::new(0),
            b"k",
            b"v",
            narrata_storage::Expect::Absent,
        );
        assert_eq!(backend.apply(&batch), Err(StorageError::Full));
        assert_eq!(backend.get_object(&digest).unwrap(), None);
        assert_eq!(backend.read_key(KeySpace::new(0), b"k").unwrap(), None);
    }
}
