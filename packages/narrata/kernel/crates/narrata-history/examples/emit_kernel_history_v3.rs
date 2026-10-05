//! Emits new shared-coordination compatibility evidence into an empty directory (ADR 0020).
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

#[path = "../tests/coordination_corpus/mod.rs"]
mod corpus;

use narrata_history::{DomainKinds, History, testing::Counter};
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
    let mut history = History::open(
        SqliteBackend::open(output.join("history.sqlite3")).unwrap(),
        DomainKinds::<Counter>::new(),
    )
    .unwrap();
    let recorded = corpus::record(&mut history);
    assert!(history.integrity_scan().unwrap().is_empty());
    let bundle = history
        .export(recorded.continued, &BTreeSet::new())
        .unwrap()
        .to_bytes()
        .unwrap();
    std::fs::write(output.join("checkpoint.bundle"), &bundle).unwrap();
    let image = corpus::image(&history);
    drop(history);
    let database = std::fs::read(output.join("history.sqlite3")).unwrap();
    let manifest = json!({"corpus_version":3,"adr":"0020",
        "artifacts":{
            "history.sqlite3":{"bytes":database.len(),"sha256":hex::encode(sha256(&database))},
            "checkpoint.bundle":{"bytes":bundle.len(),"sha256":hex::encode(sha256(&bundle))}},
        "source":recorded.source.to_string(),"migrated":recorded.migrated.to_string(),"continued":recorded.continued.to_string(),
        "target_state":12,"image":image,
        "build_provenance":{"writer":"narrata-history","domain":"Counter ceilings 1000/2000","example":"emit_kernel_history_v3"}});
    std::fs::write(
        output.join("manifest.json"),
        format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
    )
    .unwrap();
}
