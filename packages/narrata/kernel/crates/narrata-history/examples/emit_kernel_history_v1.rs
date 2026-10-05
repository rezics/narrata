//! Writes the frozen kernel history corpus under `fixtures/compat/kernel-history-v1`: a SQLite
//! store holding one counter-domain session in the generic commit format, a checkpoint bundle of
//! its branch head, and a manifest of every object and key.
//!
//! `cargo run -p narrata-history --example emit_kernel_history_v1 -- <output-directory>`

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

#[path = "../tests/corpus/mod.rs"]
mod corpus;

use std::{collections::BTreeSet, path::PathBuf};

use corpus::{BUNDLE, COUNTER, DATABASE, image, record};
use narrata_history::{
    DomainKinds, History, Pin, Transaction,
    testing::{COUNTER_COMMIT_KIND, COUNTER_INPUT_KIND, COUNTER_STATE_KIND},
};
use narrata_kernel::codec::sha256;
use narrata_storage_sqlite::SqliteBackend;
use serde_json::json;

fn main() {
    let output = PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&output).unwrap();
    let path = output.join(DATABASE);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }

    let mut history =
        History::open(SqliteBackend::open(&path).unwrap(), DomainKinds::new()).unwrap();
    let recorded = record(&mut history);
    history
        .write(
            &Transaction {
                pins: vec![Pin {
                    owner: "corpus-pin".to_owned(),
                    object: recorded.main[2],
                    expires_at: Some(1_000),
                }],
                observed_at: 9,
                ..Transaction::default()
            },
            |_| Ok(Vec::new()),
        )
        .unwrap();
    let checkpoint = history.export(recorded.branch, &BTreeSet::new()).unwrap();
    let bundle = checkpoint.to_bytes().unwrap();
    let image = image(&history);
    let integrity = history.integrity_scan().unwrap();
    assert!(integrity.is_empty(), "{integrity:?}");
    drop(history);
    std::fs::write(output.join(BUNDLE), &bundle).unwrap();

    let database = std::fs::read(&path).unwrap();
    let manifest = json!({
        "corpus_version": 1,
        "format": "narrata kernel history: save layout v1 with the transitions index and the generic commit format (ADR 0015), SQLite backend",
        "artifacts": {
            DATABASE: { "sha256": hex::encode(sha256(&database)) },
            BUNDLE: { "sha256": hex::encode(sha256(&bundle)) },
        },
        "domain": {
            "name": "counter test domain",
            "ceiling": COUNTER.ceiling(),
            "artifact": narrata_history::Domain::artifact_id(&COUNTER).to_string(),
            "kinds": {
                "commit": COUNTER_COMMIT_KIND,
                "state": COUNTER_STATE_KIND,
                "input": COUNTER_INPUT_KIND,
            },
        },
        "session": {
            "name": "player",
            "root": recorded.root.to_string(),
            "main": recorded.main.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "main_states": [0, 1, 3, 6],
            "branch_head": recorded.branch.to_string(),
            "branch_state": 6,
            "cursor": recorded.main[3].to_string(),
            "save_slot": recorded.slot.storage_key(),
        },
        "bundle": {
            "root": recorded.branch.to_string(),
            "objects": checkpoint.manifest.objects.len(),
        },
        "image": image,
        "build_provenance": {
            "writer": "narrata-history/0.1.0-alpha.1 save layout v1",
            "source": "narrata_history::testing::Counter",
            "command": "cargo run -p narrata-history --example emit_kernel_history_v1 -- fixtures/compat/kernel-history-v1",
        },
    });
    std::fs::write(
        output.join("manifest.json"),
        format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
    )
    .unwrap();
}
