//! The kernel history corpus: how it is built and how a store is described for comparison.
//! Shared by the corpus generator, which includes this file by path, and its compatibility test.

#![allow(dead_code, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use narrata_history::{
    DomainKinds, History, ObjectId, RefKey, RefName, Session, layout,
    testing::{Counter, Overflow},
};
use narrata_kernel::codec::sha256;
use narrata_storage::{ObjectDigest, StorageBackend};
use serde_json::{Value as Json, json};

pub const DATABASE: &str = "history.sqlite3";
pub const BUNDLE: &str = "checkpoint.bundle";
pub const COUNTER: Counter = Counter::new(1_000);

const PAGE: u32 = 64;

pub type Counters<B> = History<B, DomainKinds<Counter>>;

pub fn name(value: &str) -> RefName {
    RefName::new(value).unwrap()
}

pub fn step(counter: &Counter, state: &u64, input: &u64) -> Result<u64, Overflow> {
    counter.step(state, input)
}

/// The session the corpus records: a main line 0 → 1 → 3 → 6, a branch 1 → 6 from the first
/// commit, a save slot on the branch head and the cursor back on the main head.
pub struct Recorded {
    pub root: ObjectId,
    pub main: Vec<ObjectId>,
    pub branch: ObjectId,
    pub slot: RefKey,
}

pub fn record<B: StorageBackend>(history: &mut Counters<B>) -> Recorded {
    let (mut session, root) = Session::create(history, COUNTER, name("player"), &0, 1).unwrap();
    let mut main = vec![root.commit];
    for (time, increment) in [(2, 1), (3, 2), (4, 3)] {
        let head = session.head();
        main.push(
            session
                .advance(history, head, &increment, step, time)
                .unwrap()
                .commit,
        );
    }
    session.checkout(history, main[3], main[1], 5).unwrap();
    let branch = session
        .advance(history, main[1], &5, step, 6)
        .unwrap()
        .commit;
    let slot = RefKey::save(name("player"), name("slot-1"));
    session.save(history, slot.clone(), None, 7).unwrap();
    session.checkout(history, branch, main[3], 8).unwrap();
    Recorded {
        root: root.commit,
        main,
        branch,
        slot,
    }
}

/// Every stored object and every key of the history layer's spaces, as stored.
pub fn image<B: StorageBackend>(history: &Counters<B>) -> Json {
    let backend = history.backend();
    let mut digests = Vec::<ObjectDigest>::new();
    let mut after = None;
    loop {
        let page = backend.scan_objects(after.as_ref(), PAGE).unwrap();
        after = page.resume_after().copied();
        digests.extend(page.digests);
        if after.is_none() {
            break;
        }
    }
    let ids = digests
        .iter()
        .map(|digest| ObjectId::from_bytes(*digest.as_bytes()))
        .collect::<Vec<_>>();
    let objects = history
        .get_objects(&ids)
        .unwrap()
        .into_iter()
        .map(|object| {
            let object = object.unwrap();
            json!({
                "id": object.id().to_string(),
                "kind": object.kind(),
                "schema": object.schema(),
                "sha256": hex::encode(sha256(object.bytes())),
            })
        })
        .collect::<Vec<_>>();
    let mut keys = Vec::new();
    for (space, name) in layout::SPACES {
        let mut after: Option<Vec<u8>> = None;
        loop {
            let page = backend
                .scan_keys(space, b"", after.as_deref(), PAGE)
                .unwrap();
            after = page.resume_after().map(<[u8]>::to_vec);
            keys.extend(page.entries.into_iter().map(|entry| {
                json!({
                    "space": name,
                    "key": hex::encode(&entry.key),
                    "value": hex::encode(&entry.value),
                    "revision": entry.revision.get(),
                })
            }));
            if after.is_none() {
                break;
            }
        }
    }
    json!({ "objects": objects, "keys": keys })
}

/// The corpus directory `name` under `fixtures/compat`.
pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../fixtures/compat")
        .join(name)
}
