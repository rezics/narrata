//! The frozen kernel history corpus still opens, loads, imports and continues (ADR 0015).

#![allow(clippy::panic, clippy::unwrap_used)]

mod corpus;

use std::{collections::BTreeSet, path::PathBuf, str::FromStr};

use corpus::{BUNDLE, COUNTER, Counters, DATABASE, fixture, image, name, record, step};
use narrata_history::{
    BundleLimits, Domain, DomainKinds, History, ObjectId, RefKey, Session,
    testing::{Counter, Overflow},
};
use narrata_kernel::codec::sha256;
use narrata_storage::MemoryBackend;
use narrata_storage_sqlite::SqliteBackend;
use serde_json::Value as Json;
use tempfile::TempDir;

const CORPUS: &str = "kernel-history-v1";

fn manifest() -> Json {
    serde_json::from_slice(&std::fs::read(fixture(CORPUS).join("manifest.json")).unwrap()).unwrap()
}

/// The bytes of a corpus artifact after checking they are the recorded ones.
fn artifact(manifest: &Json, file: &str) -> Vec<u8> {
    let path = fixture(CORPUS).join(file);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        hex::encode(sha256(&bytes)),
        manifest["artifacts"][file]["sha256"].as_str().unwrap(),
        "{} differs from its manifest",
        path.display()
    );
    bytes
}

/// A scratch copy of the frozen database; frozen corpora are never opened in place.
fn copy_database(manifest: &Json) -> (TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let copy = directory.path().join(DATABASE);
    std::fs::write(&copy, artifact(manifest, DATABASE)).unwrap();
    (directory, copy)
}

fn never(_: &Counter, _: &u64, _: &u64) -> Result<u64, Overflow> {
    unreachable!("a recorded transition ran its step")
}

fn id(value: &Json) -> ObjectId {
    ObjectId::from_str(value.as_str().unwrap()).unwrap()
}

#[test]
fn the_corpus_records_the_counter_domain_this_build_defines() {
    let manifest = manifest();
    assert_eq!(
        manifest["domain"]["artifact"].as_str().unwrap(),
        COUNTER.artifact_id().to_string()
    );
    // The recorded session is what the generator records today, object for object.
    let mut history = History::in_memory(DomainKinds::<Counter>::new());
    let recorded = record(&mut history);
    let session = &manifest["session"];
    assert_eq!(recorded.root, id(&session["root"]));
    assert_eq!(recorded.branch, id(&session["branch_head"]));
    let main = session["main"]
        .as_array()
        .unwrap()
        .iter()
        .map(id)
        .collect::<Vec<_>>();
    assert_eq!(recorded.main, main);
    assert_eq!(recorded.slot.storage_key(), session["save_slot"]);
}

#[test]
fn frozen_store_opens_unchanged_loads_and_continues() {
    let manifest = manifest();
    let session = &manifest["session"];
    let (_directory, path) = copy_database(&manifest);
    let mut history: Counters<SqliteBackend> =
        History::open(SqliteBackend::open(&path).unwrap(), DomainKinds::new()).unwrap();
    assert_eq!(image(&history), manifest["image"]);
    assert!(history.integrity_scan().unwrap().is_empty());

    // Every recorded commit loads with its recorded state, without replay.
    let main = session["main"].as_array().unwrap();
    for (commit, state) in main.iter().zip(session["main_states"].as_array().unwrap()) {
        let loaded = history.load(&COUNTER, id(commit)).unwrap();
        assert_eq!(loaded.state, state.as_u64().unwrap());
    }
    history
        .verify_path(&COUNTER, id(&session["branch_head"]), step)
        .unwrap();

    let (mut player, loaded) = Session::open(&history, COUNTER, name("player")).unwrap();
    assert_eq!(player.head(), id(&session["cursor"]));
    assert_eq!(loaded.state, 6);
    let slot = RefKey::save(name("player"), name("slot-1"));
    let saved = player.load_save(&mut history, &slot, 20).unwrap();
    assert_eq!(saved.commit, id(&session["branch_head"]));

    // Recorded transitions are reused; new ones are recorded and survive GC.
    let root = id(&session["root"]);
    player
        .checkout(&mut history, saved.commit, root, 21)
        .unwrap();
    let reused = player.advance(&mut history, root, &1, never, 22).unwrap();
    assert_eq!((reused.commit, reused.reused), (id(&main[1]), true));
    let next = player
        .advance(&mut history, reused.commit, &7, step, 23)
        .unwrap();
    assert_eq!((next.state, next.depth, next.reused), (8, 2, false));
    history.collect(Default::default()).unwrap();
    assert!(history.integrity_scan().unwrap().is_empty());
    assert_eq!(history.load(&COUNTER, next.commit).unwrap().state, 8);
}

#[test]
fn frozen_checkpoint_imports_checked() {
    let manifest = manifest();
    let bytes = artifact(&manifest, BUNDLE);
    let root = id(&manifest["bundle"]["root"]);
    let mut history = History::open(MemoryBackend::new(), DomainKinds::<Counter>::new()).unwrap();
    let slot = RefKey::save(name("imported"), name("slot"));
    let (value, loaded) = history
        .import(&COUNTER, &bytes, BundleLimits::default(), slot, None, 1)
        .unwrap();
    assert_eq!((value.commit, loaded.state), (root, 6));
    let exported = history.export(root, &BTreeSet::new()).unwrap();
    assert_eq!(
        exported.manifest.objects.len() as u64,
        manifest["bundle"]["objects"].as_u64().unwrap()
    );
    // Re-exporting what was imported reproduces the frozen bytes.
    assert_eq!(exported.to_bytes().unwrap(), bytes);
    // Another artifact refuses the bundle.
    let mut other = History::open(MemoryBackend::new(), DomainKinds::<Counter>::new()).unwrap();
    assert!(
        other
            .import(
                &Counter::new(10),
                &bytes,
                BundleLimits::default(),
                RefKey::save(name("imported"), name("slot")),
                None,
                1
            )
            .is_err()
    );
}
