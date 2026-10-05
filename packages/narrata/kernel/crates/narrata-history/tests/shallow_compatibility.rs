//! Both the v2 bytes and imported SQLite image are frozen read-only evidence (ADR 0019).
#![allow(clippy::panic, clippy::unwrap_used)]
mod corpus;

use corpus::{COUNTER, fixture, image, name, step};
use narrata_history::{BundleLimits, DomainKinds, History, HistoryError, ObjectId, RefKey};
use narrata_kernel::codec::sha256;
use narrata_storage_sqlite::SqliteBackend;
use serde_json::Value;
use std::{collections::BTreeSet, str::FromStr};

fn manifest() -> Value {
    serde_json::from_slice(
        &std::fs::read(fixture("kernel-history-v2").join("manifest.json")).unwrap(),
    )
    .unwrap()
}

fn artifact(manifest: &Value, file: &str) -> Vec<u8> {
    let bytes = std::fs::read(fixture("kernel-history-v2").join(file)).unwrap();
    assert_eq!(
        hex::encode(sha256(&bytes)),
        manifest["artifacts"][file]["sha256"]
    );
    assert_eq!(bytes.len() as u64, manifest["artifacts"][file]["bytes"]);
    bytes
}

fn id(value: &Value) -> ObjectId {
    ObjectId::from_str(value.as_str().unwrap()).unwrap()
}

#[test]
fn frozen_shallow_bundles_import_reexport_and_hydrate() {
    let manifest = manifest();
    let target = id(&manifest["target"]);
    for (file, ancestors, boundary) in [("shallow-0.bundle", 0, 2), ("shallow-1.bundle", 1, 1)] {
        let bytes = artifact(&manifest, file);
        let mut history =
            History::in_memory(DomainKinds::<narrata_history::testing::Counter>::new());
        let slot = RefKey::save(name("restored"), name("slot"));
        let (value, loaded) = history
            .import(
                &COUNTER,
                &bytes,
                BundleLimits::default(),
                slot.clone(),
                None,
                10,
            )
            .unwrap();
        assert_eq!((loaded.commit, loaded.state), (target, 6));
        let parent = id(&manifest["commits"][boundary]);
        assert_eq!(
            history.load(&COUNTER, parent),
            Err(HistoryError::HistoryTruncated(parent))
        );
        history.collect(Default::default()).unwrap();
        assert!(history.integrity_scan().unwrap().is_empty());
        assert_eq!(
            history
                .export_shallow(target, ancestors, &BTreeSet::new())
                .unwrap()
                .to_bytes()
                .unwrap(),
            bytes
        );
        let complete = artifact(&manifest, "complete.bundle");
        history
            .import(
                &COUNTER,
                &complete,
                BundleLimits::default(),
                slot,
                Some(value.revision),
                11,
            )
            .unwrap();
        history.verify_path(&COUNTER, target, step).unwrap();
        assert_eq!(
            history
                .export(target, &BTreeSet::new())
                .unwrap()
                .to_bytes()
                .unwrap(),
            complete
        );
    }
}

#[test]
fn frozen_shallow_sqlite_reopens_without_modifying_the_corpus() {
    let manifest = manifest();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.sqlite3");
    std::fs::write(&path, artifact(&manifest, "history.sqlite3")).unwrap();
    let mut history = History::open(
        SqliteBackend::open(&path).unwrap(),
        DomainKinds::<narrata_history::testing::Counter>::new(),
    )
    .unwrap();
    assert_eq!(image(&history), manifest["image"]);
    let target = id(&manifest["target"]);
    assert_eq!(history.load(&COUNTER, target).unwrap().state, 6);
    let parent = id(&manifest["commits"][1]);
    assert_eq!(
        history.load(&COUNTER, parent),
        Err(HistoryError::HistoryTruncated(parent))
    );
    history.collect(Default::default()).unwrap();
    assert!(history.integrity_scan().unwrap().is_empty());
    drop(history);
    let history = History::open(
        SqliteBackend::open(&path).unwrap(),
        DomainKinds::<narrata_history::testing::Counter>::new(),
    )
    .unwrap();
    assert_eq!(
        history
            .export_shallow(target, 1, &BTreeSet::new())
            .unwrap()
            .to_bytes()
            .unwrap(),
        artifact(&manifest, "shallow-1.bundle")
    );
}
