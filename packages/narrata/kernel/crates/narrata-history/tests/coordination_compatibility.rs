//! The v3 database and artifact-changing checkpoint stay frozen read-only (ADR 0020).
#![allow(clippy::panic, clippy::unwrap_used)]
mod coordination_corpus;

use coordination_corpus::{Effects, SOURCE, TARGET, claim, fixture, image, name, step};
use narrata_history::{
    BundleLimits, DomainKinds, History, ObjectId, RefKey, Session,
    effect::{EffectClaimResult, EffectId, ExecutionId, LedgerStatus},
    testing::Counter,
};
use narrata_kernel::codec::sha256;
use narrata_storage_sqlite::SqliteBackend;
use serde_json::Value;
use std::{collections::BTreeSet, str::FromStr};

fn manifest() -> Value {
    serde_json::from_slice(
        &std::fs::read(fixture("kernel-history-v3").join("manifest.json")).unwrap(),
    )
    .unwrap()
}
fn artifact(manifest: &Value, file: &str) -> Vec<u8> {
    let bytes = std::fs::read(fixture("kernel-history-v3").join(file)).unwrap();
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
fn frozen_shared_ledger_and_migration_open_and_continue() {
    let manifest = manifest();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.sqlite3");
    std::fs::write(&path, artifact(&manifest, "history.sqlite3")).unwrap();
    let mut history = History::open(
        SqliteBackend::open(&path).unwrap(),
        DomainKinds::<Counter>::new(),
    )
    .unwrap();
    assert_eq!(image(&history), manifest["image"]);
    let source = id(&manifest["source"]);
    let continued = id(&manifest["continued"]);
    assert_eq!(history.load(&SOURCE, source).unwrap().state, 1);
    assert_eq!(history.load(&TARGET, continued).unwrap().state, 12);
    let entry = history
        .read_effect_entry::<Effects>(ExecutionId::from_u128(1), EffectId::from_bytes([1; 32]))
        .unwrap()
        .unwrap()
        .0;
    assert!(
        matches!(entry.status,LedgerStatus::Compensated {original_fence,compensation_fence,..} if original_fence.get()==1 && compensation_fence.get()==2)
    );
    let unknown = history
        .read_effect_entry::<Effects>(ExecutionId::from_u128(1), EffectId::from_bytes([4; 32]))
        .unwrap()
        .unwrap()
        .0;
    let mut c = claim(source, 4);
    c.rewind = narrata_history::effect::RewindPolicy::Barrier;
    assert_eq!(
        history.claim_effect::<Effects>(c).unwrap(),
        EffectClaimResult::Recorded(unknown)
    );
    let (mut session, _) = Session::open(&history, TARGET, name("target")).unwrap();
    let next = session
        .advance(&mut history, continued, &1, step, 6)
        .unwrap();
    assert_eq!(next.state, 13);
    assert!(history.integrity_scan().unwrap().is_empty());
}

#[test]
fn frozen_migration_bundle_imports_reexports_and_continues() {
    let manifest = manifest();
    let bytes = artifact(&manifest, "checkpoint.bundle");
    let mut history = History::in_memory(DomainKinds::<Counter>::new());
    let (_, loaded) = history
        .import(
            &TARGET,
            &bytes,
            BundleLimits::default(),
            RefKey::active(name("restored")).unwrap(),
            None,
            6,
        )
        .unwrap();
    assert_eq!(loaded.state, 12);
    assert_eq!(loaded.commit, id(&manifest["continued"]));
    assert_eq!(
        history
            .export(loaded.commit, &BTreeSet::new())
            .unwrap()
            .to_bytes()
            .unwrap(),
        bytes
    );
    assert_eq!(
        history
            .load(&SOURCE, id(&manifest["source"]))
            .unwrap()
            .state,
        1
    );
    let (mut session, _) = Session::open(&history, TARGET, name("restored")).unwrap();
    assert_eq!(
        session
            .advance(&mut history, loaded.commit, &1, step, 7)
            .unwrap()
            .state,
        13
    );
    history.collect(Default::default()).unwrap();
    assert!(history.integrity_scan().unwrap().is_empty());
}

#[test]
fn shared_coordination_writer_is_deterministic() {
    let manifest = manifest();
    let mut first = History::in_memory(DomainKinds::<Counter>::new());
    let mut second = History::in_memory(DomainKinds::<Counter>::new());
    let one = coordination_corpus::record(&mut first);
    let two = coordination_corpus::record(&mut second);
    assert_eq!(one.continued, two.continued);
    assert_eq!(image(&first), image(&second));
    assert_eq!(image(&first), manifest["image"]);
    assert_eq!(
        first
            .export(one.continued, &BTreeSet::new())
            .unwrap()
            .to_bytes()
            .unwrap(),
        artifact(&manifest, "checkpoint.bundle")
    );
}
