//! SQLite saves: the save engine over the row-level `narrata-storage-sqlite` backend
//! (ADR 0014), plus the explicit migration from schema v2 databases.
//!
//! The backend runs every batch in one `BEGIN IMMEDIATE` transaction under WAL with
//! `synchronous=FULL`; callers should still verify the guarantees of their filesystem.

#![forbid(unsafe_code)]

mod v2;

use std::path::Path;

use narrata_storage::StorageError;
pub use narrata_storage_sqlite::{SqliteBackend, SqliteOptions};
use narrata_store::{Store, StoreError};

/// A save store in a SQLite file.
pub type SqliteStore = Store<SqliteBackend>;

/// Opens or creates the save store at `path`. A schema v2 database is refused, unmodified, with
/// a format error that names [`migrate_v2`].
pub fn open(path: impl AsRef<Path>) -> Result<SqliteStore, StoreError> {
    open_with(path, SqliteOptions::default())
}

pub fn open_with(
    path: impl AsRef<Path>,
    options: SqliteOptions,
) -> Result<SqliteStore, StoreError> {
    let path = path.as_ref();
    if let Some(version) = v2::schema_version(path) {
        return Err(StorageError::Format(format!(
            "{} is a schema v{version} save database; migrate it with \
             `narrata store migrate-v2` or narrata_store_sqlite::migrate_v2",
            path.display()
        ))
        .into());
    }
    Store::open(SqliteBackend::open_with(path, options)?)
}

/// Opens a save store that lives only as long as the returned value.
pub fn open_in_memory() -> Result<SqliteStore, StoreError> {
    Store::open(SqliteBackend::open(":memory:")?)
}

/// Copies the saves of a schema v2 database at `source` into a new store at `target`.
///
/// The source is opened read-only and left untouched. Objects keep their bytes and insertion
/// times; Refs, Catalog Heads, the Effect ledger, fences, inputs and Pins keep their values and
/// receive new revisions. `target` must not exist; if migration fails, the partial target is
/// removed.
pub fn migrate_v2(
    source: impl AsRef<Path>,
    target: impl AsRef<Path>,
) -> Result<SqliteStore, StoreError> {
    let (source, target) = (source.as_ref(), target.as_ref());
    let contents = v2::read(source)?;
    if target.exists() {
        return Err(StorageError::Invalid("migration target already exists").into());
    }
    let restored = SqliteBackend::open(target)
        .map_err(StoreError::from)
        .and_then(|backend| Store::restore(backend, contents));
    if restored.is_err() {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", target.display()));
        }
    }
    restored
}
