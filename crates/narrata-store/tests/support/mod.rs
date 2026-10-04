//! Backends the store tests run on, and what the tests observe of them.
//!
//! Each integration test that includes this module uses only part of it.
#![allow(dead_code, clippy::panic, clippy::unwrap_used)]

use std::{fmt::Debug, sync::Arc};

use narrata_core::{
    CheckedProgram, ExecutionId,
    program::{OpV0, encode_program_artifact, load_program},
};
use narrata_storage::{
    Limits, MemoryBackend, ObjectDigest, StorageBackend, StorageError,
    testing::{Fault, FaultInjecting, FaultPlan, Trigger},
};
use narrata_storage_sqlite::{SqliteBackend, SqliteOptions};
use narrata_store::{
    EffectLedgerEntry, RefKey, RefScope, RefValue, SaveStore, Store, layout, scan_all,
};
use narrata_testkit::generator::hello_v0;
use tempfile::TempDir;

/// A storage backend the engine tests run against.
pub trait Backend {
    type Inner: StorageBackend + Send + 'static;

    /// A new, empty backend.
    fn create() -> Self::Inner;

    /// A new, empty backend that enforces `limits`.
    fn with_limits(limits: Limits) -> Self::Inner;

    /// Two live handles to one new, empty backend, and whatever keeps it alive.
    fn pair() -> (Self::Inner, Self::Inner, Option<TempDir>);

    fn store() -> Store<Self::Inner> {
        Store::open(Self::create()).unwrap()
    }

    /// A store whose backend fails the calls its plan names; the plan starts empty.
    fn faulty() -> Store<FaultInjecting<Self::Inner>> {
        Store::open(FaultInjecting::new(Self::create(), FaultPlan::new())).unwrap()
    }
}

/// The in-memory reference backend.
pub struct Memory;

impl Backend for Memory {
    type Inner = MemoryBackend;

    fn create() -> MemoryBackend {
        MemoryBackend::new()
    }

    fn with_limits(limits: Limits) -> MemoryBackend {
        MemoryBackend::with_limits(limits)
    }

    fn pair() -> (MemoryBackend, MemoryBackend, Option<TempDir>) {
        let backend = MemoryBackend::new();
        (backend.share(), backend, None)
    }
}

/// The row-level SQLite backend.
pub struct Sqlite;

impl Backend for Sqlite {
    type Inner = SqliteBackend;

    fn create() -> SqliteBackend {
        SqliteBackend::open(":memory:").unwrap()
    }

    fn with_limits(limits: Limits) -> SqliteBackend {
        SqliteBackend::open_with(
            ":memory:",
            SqliteOptions {
                limits,
                ..SqliteOptions::default()
            },
        )
        .unwrap()
    }

    fn pair() -> (SqliteBackend, SqliteBackend, Option<TempDir>) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("saves.sqlite3");
        let first = SqliteBackend::open(&path).unwrap();
        let second = SqliteBackend::open(&path).unwrap();
        (first, second, Some(directory))
    }
}

/// Expands each named generic test `fn name<B: Backend>()` into one `#[test]` per backend.
#[allow(unused_macros)]
macro_rules! backend_tests {
    ($($name:ident),* $(,)?) => {
        mod memory {
            $(
                #[test]
                fn $name() {
                    super::$name::<crate::support::Memory>();
                }
            )*
        }
        mod sqlite {
            $(
                #[test]
                fn $name() {
                    super::$name::<crate::support::Sqlite>();
                }
            )*
        }
    };
}

/// Everything stored: each object with its bytes and each key in every layout space with its
/// value and revision, read through small paged scans.
#[derive(Debug, Eq, PartialEq)]
pub struct Image {
    pub objects: Vec<(ObjectDigest, Vec<u8>)>,
    pub keys: Vec<(u16, Vec<u8>, Vec<u8>, u64)>,
}

impl Image {
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// The keys of one space.
    pub fn space(&self, space: narrata_storage::KeySpace) -> Vec<&(u16, Vec<u8>, Vec<u8>, u64)> {
        self.keys
            .iter()
            .filter(|(number, ..)| *number == space.get())
            .collect()
    }
}

const PAGE: u32 = 7;

pub fn image(backend: &impl StorageBackend) -> Image {
    let mut digests = Vec::new();
    let mut after = None;
    loop {
        let page = backend.scan_objects(after.as_ref(), PAGE).unwrap();
        after = page.resume_after().copied();
        digests.extend(page.digests);
        if after.is_none() {
            break;
        }
    }
    let objects = digests
        .chunks(PAGE as usize)
        .flat_map(|chunk| {
            let found = backend.get_objects(chunk).unwrap();
            chunk.iter().copied().zip(found).collect::<Vec<_>>()
        })
        .map(|(digest, bytes)| (digest, bytes.unwrap().to_vec()))
        .collect();
    let mut keys = Vec::new();
    for (space, _) in layout::SPACES {
        let mut after: Option<Vec<u8>> = None;
        loop {
            let page = backend
                .scan_keys(space, b"", after.as_deref(), PAGE)
                .unwrap();
            after = page.resume_after().map(<[u8]>::to_vec);
            keys.extend(
                page.entries
                    .into_iter()
                    .map(|entry| (space.get(), entry.key, entry.value, entry.revision.get())),
            );
            if after.is_none() {
                break;
            }
        }
    }
    Image { objects, keys }
}

/// Runs `operation` on a fresh `setup` once per backend call it makes, failing that call before
/// it takes effect and, separately, after it took effect. A failed run must leave the backend
/// exactly as it was; a run that succeeds despite the fault, because the engine reconciled the
/// unknown outcome, must leave it exactly as the fault-free run does. Returns how many calls the
/// operation makes.
pub fn assert_atomic<T, B, R, E>(
    mut setup: impl FnMut() -> T,
    store: impl Fn(&mut T) -> &mut Store<FaultInjecting<B>>,
    mut operation: impl FnMut(&mut T) -> Result<R, E>,
) -> u64
where
    B: StorageBackend,
    E: Debug,
{
    let mut reference = setup();
    store(&mut reference)
        .backend_mut()
        .set_plan(FaultPlan::new());
    if let Err(error) = operation(&mut reference) {
        panic!("the fault-free run failed: {error:?}");
    }
    let calls = store(&mut reference).backend().calls();
    let expected = image(store(&mut reference).backend().inner());
    for call in 1..=calls {
        for fault in [
            Fault::Before(StorageError::Io("injected".to_owned())),
            Fault::After,
        ] {
            let mut state = setup();
            let before = image(store(&mut state).backend().inner());
            store(&mut state)
                .backend_mut()
                .set_plan(FaultPlan::new().at(Trigger::Call(call), fault.clone()));
            let result = operation(&mut state);
            let after = image(store(&mut state).backend().inner());
            if result.is_ok() {
                assert_eq!(
                    after, expected,
                    "{fault:?} at call {call} of {calls} succeeded"
                );
            } else {
                assert_eq!(after, before, "{fault:?} at call {call} of {calls} failed");
            }
        }
    }
    calls
}

/// Every Effect ledger entry of `execution`.
pub fn effects(store: &impl SaveStore, execution: ExecutionId) -> Vec<EffectLedgerEntry> {
    scan_all(
        |after| store.scan_effects(execution, after, 3),
        |entry: &EffectLedgerEntry| entry.effect,
    )
    .unwrap()
}

/// Every Ref.
pub fn refs(store: &impl SaveStore) -> Vec<(RefKey, RefValue)> {
    scan_all(
        |after| store.scan_refs(&RefScope::All, after, 3),
        |(key, _): &(RefKey, RefValue)| key.clone(),
    )
    .unwrap()
}

/// A Program that yields on every input, indefinitely, without growing its state.
pub fn looping_program() -> Arc<CheckedProgram> {
    let mut artifact = hello_v0();
    let say = artifact.flows[0].entry;
    let OpV0::Say { next, .. } = &mut artifact.flows[0].instructions[0].op else {
        panic!("hello generator must begin with Say");
    };
    *next = say;
    artifact.flows[0].instructions.truncate(1);
    load_program(&encode_program_artifact(&artifact), &Default::default()).unwrap()
}

pub fn program(artifact: &narrata_core::program::ProgramArtifactV0) -> Arc<CheckedProgram> {
    load_program(&encode_program_artifact(artifact), &Default::default()).unwrap()
}
