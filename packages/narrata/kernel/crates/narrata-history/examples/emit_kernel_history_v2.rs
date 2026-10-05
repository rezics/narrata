//! Emit a new shallow-history compatibility corpus into an empty output directory (ADR 0019).
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

#[path = "../tests/corpus/mod.rs"]
mod corpus;

use corpus::{COUNTER, image, name, record};
use narrata_history::{BundleLimits, DomainKinds, History, RefKey};
use narrata_kernel::codec::sha256;
use narrata_storage_sqlite::SqliteBackend;
use serde_json::json;
use std::{collections::BTreeSet, path::PathBuf};

fn main() {
    let output = PathBuf::from(std::env::args().nth(1).expect("empty output directory"));
    std::fs::create_dir_all(&output).unwrap();
    assert!(
        std::fs::read_dir(&output).unwrap().next().is_none(),
        "never overwrite a frozen corpus"
    );
    let mut source = History::in_memory(DomainKinds::new());
    let recorded = record(&mut source);
    let target = recorded.main[3];
    let mut artifacts = serde_json::Map::new();
    for (file, bytes) in [
        (
            "shallow-0.bundle",
            source
                .export_shallow(target, 0, &BTreeSet::new())
                .unwrap()
                .to_bytes()
                .unwrap(),
        ),
        (
            "shallow-1.bundle",
            source
                .export_shallow(target, 1, &BTreeSet::new())
                .unwrap()
                .to_bytes()
                .unwrap(),
        ),
        (
            "complete.bundle",
            source
                .export(target, &BTreeSet::new())
                .unwrap()
                .to_bytes()
                .unwrap(),
        ),
    ] {
        std::fs::write(output.join(file), &bytes).unwrap();
        artifacts.insert(
            file.to_owned(),
            json!({ "bytes": bytes.len(), "sha256": hex::encode(sha256(&bytes)) }),
        );
    }
    let mut imported = History::open(
        SqliteBackend::open(output.join("history.sqlite3")).unwrap(),
        DomainKinds::<narrata_history::testing::Counter>::new(),
    )
    .unwrap();
    imported
        .import(
            &COUNTER,
            &std::fs::read(output.join("shallow-1.bundle")).unwrap(),
            BundleLimits::default(),
            RefKey::save(name("restored"), name("slot")),
            None,
            10,
        )
        .unwrap();
    assert!(imported.integrity_scan().unwrap().is_empty());
    let stored_image = image(&imported);
    drop(imported);
    let database = std::fs::read(output.join("history.sqlite3")).unwrap();
    artifacts.insert(
        "history.sqlite3".to_owned(),
        json!({ "bytes": database.len(), "sha256": hex::encode(sha256(&database)) }),
    );
    let manifest = json!({
        "corpus_version": 2, "adr": "0019", "artifacts": artifacts,
        "target": target.to_string(), "target_state": 6,
        "commits": recorded.main.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "image": stored_image,
        "build_provenance": { "writer": "narrata-history", "domain": "Counter ceiling 1000", "example": "emit_kernel_history_v2" },
    });
    std::fs::write(
        output.join("manifest.json"),
        format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
    )
    .unwrap();
}
