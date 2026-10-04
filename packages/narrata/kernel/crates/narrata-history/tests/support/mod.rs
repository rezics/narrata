//! Stores for the history tests: every scenario runs on the memory backend and on SQLite.

#![allow(dead_code, clippy::unwrap_used)]

use std::{collections::BTreeMap, path::PathBuf};

use narrata_history::{
    BranchId, DomainKinds, History, ObjectId, RefKey, RefName, RefNamespace, RefScope, RefValue,
    Session, scan_all,
    testing::{Counter, Overflow},
};
use narrata_storage::{MemoryBackend, StorageBackend};
use narrata_storage_sqlite::SqliteBackend;
use tempfile::TempDir;

/// One store; every call to [`Fixture::backend`] opens another handle on it.
pub trait Fixture {
    type Backend: StorageBackend;

    fn new() -> Self;

    fn backend(&self) -> Self::Backend;
}

pub struct Memory(MemoryBackend);

impl Fixture for Memory {
    type Backend = MemoryBackend;

    fn new() -> Self {
        Self(MemoryBackend::new())
    }

    fn backend(&self) -> MemoryBackend {
        self.0.share()
    }
}

pub struct Sqlite {
    _directory: TempDir,
    path: PathBuf,
}

impl Fixture for Sqlite {
    type Backend = SqliteBackend;

    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        Self {
            _directory: directory,
            path,
        }
    }

    fn backend(&self) -> SqliteBackend {
        SqliteBackend::open(&self.path).unwrap()
    }
}

/// Runs a scenario generic over [`Fixture`] once per backend.
macro_rules! on_both_backends {
    ($($scenario:ident),* $(,)?) => {
        $(
            mod $scenario {
                #[test]
                fn memory() {
                    super::$scenario::<super::support::Memory>();
                }

                #[test]
                fn sqlite() {
                    super::$scenario::<super::support::Sqlite>();
                }
            }
        )*
    };
}

pub type Counters<B> = History<B, DomainKinds<Counter>>;

pub const COUNTER: Counter = Counter::new(1_000);

pub fn open<F: Fixture>(fixture: &F) -> Counters<F::Backend> {
    History::open(fixture.backend(), DomainKinds::new()).unwrap()
}

pub fn name(value: &str) -> RefName {
    RefName::new(value).unwrap()
}

pub fn step(counter: &Counter, state: &u64, input: &u64) -> Result<u64, Overflow> {
    counter.step(state, input)
}

/// A step for transitions that must already be recorded.
pub fn never(_: &Counter, _: &u64, _: &u64) -> Result<u64, Overflow> {
    unreachable!("a recorded transition ran its step")
}

/// The branch heads of session `owner`.
pub fn heads<B: StorageBackend>(
    history: &Counters<B>,
    owner: &str,
) -> BTreeMap<BranchId, ObjectId> {
    scan_all(
        |after| {
            history.scan_refs(
                &RefScope::Owner(RefNamespace::Branch, name(owner)),
                after,
                2,
            )
        },
        |(key, _): &(RefKey, RefValue)| key.clone(),
    )
    .unwrap()
    .into_iter()
    .map(|(key, value)| (key.branch_id().unwrap(), value.commit))
    .collect()
}

/// A session at state 0 advanced through `increments`, and the commits it passed, root first.
pub fn chain<B: StorageBackend>(
    history: &mut Counters<B>,
    session: &str,
    increments: &[u64],
) -> (Session<Counter>, Vec<ObjectId>) {
    let (mut session, root) = Session::create(history, COUNTER, name(session), &0, 1).unwrap();
    let mut commits = vec![root.commit];
    for (time, increment) in (2..).zip(increments) {
        let head = session.head();
        commits.push(
            session
                .advance(history, head, increment, step, time)
                .unwrap()
                .commit,
        );
    }
    (session, commits)
}
