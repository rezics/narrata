#![allow(clippy::unwrap_used)]

use narrata_storage::{Limits, testing::conformance::Harness};
use narrata_storage_sqlite::{SqliteBackend, SqliteOptions};
use tempfile::TempDir;

/// Each case gets its own directory of store files, removed when the case ends.
struct Files {
    dir: TempDir,
    created: u32,
}

impl Files {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            created: 0,
        }
    }

    fn options() -> SqliteOptions {
        // Smaller than the defaults so that the limit cases write megabytes, not gigabytes.
        SqliteOptions {
            limits: Limits {
                max_value_bytes: 256 * 1024,
                max_batch_bytes: 1024 * 1024,
                ..Limits::DEFAULT
            },
            ..SqliteOptions::default()
        }
    }
}

impl Harness for Files {
    type Backend = SqliteBackend;

    fn create(&mut self) -> SqliteBackend {
        self.created += 1;
        let path = self
            .dir
            .path()
            .join(format!("store-{}.sqlite", self.created));
        SqliteBackend::open_with(path, Self::options()).unwrap()
    }

    fn connect(&mut self, backend: &SqliteBackend) -> Option<SqliteBackend> {
        Some(SqliteBackend::open_with(backend.path(), Self::options()).unwrap())
    }

    fn reopen(&mut self, backend: SqliteBackend) -> Option<SqliteBackend> {
        let path = backend.path().to_path_buf();
        drop(backend);
        Some(SqliteBackend::open_with(path, Self::options()).unwrap())
    }
}

narrata_storage::conformance_tests!(Files::new());
