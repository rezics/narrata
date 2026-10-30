#![allow(clippy::unwrap_used, clippy::panic)]
#[path = "../../../../nodes/crates/narrata-nodes/tests/support/mod.rs"]
mod support;
use narrata_authoring::{maps::reader_map, project_relations};
use narrata_nodes::{canonical_json, history::evolution::*};
use std::{collections::BTreeMap, path::PathBuf};
use support::*;

fn directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../../fixtures/compat/reader-map-v1")
}
fn corpus() -> BTreeMap<String, Vec<u8>> {
    let compilation = compiled();
    let (mut s, names) = session(&compilation);
    let mut registry = EvolutionRegistry::new();
    registry.register_program(s.program().clone()).unwrap();
    let projection = project_relations(s.program()).unwrap();
    let mut files = BTreeMap::new();
    files.insert("story.narpack".into(), compilation.pack.clone());
    for (name, keys) in [
        ("initial", vec![]),
        ("local-disabled", vec!["wave"]),
        ("multi", vec!["rope", "lamp"]),
        ("called", vec!["left"]),
        ("ending", vec!["continue"]),
    ] {
        if !keys.is_empty() {
            choose(&mut s, &names, &keys).unwrap();
        }
        let visits = registry
            .visited_from_save(s.history(), s.cursor().unwrap(), PathLimits::default())
            .unwrap();
        files.insert(
            format!("{name}.visits.json"),
            canonical_json(&serde_json::to_value(visits.report()).unwrap()).unwrap(),
        );
        files.insert(
            format!("{name}.map.json"),
            canonical_json(
                &serde_json::to_value(reader_map(&projection, &visits).unwrap()).unwrap(),
            )
            .unwrap(),
        );
        files.insert(
            format!("{name}.checkpoint"),
            s.export().unwrap().into_bytes(),
        );
    }
    let digests: BTreeMap<_, _> = files
        .iter()
        .map(|(name, bytes)| {
            (
                name.clone(),
                hex::encode(narrata_kernel::codec::digest_bytes(
                    "narrata.reader-map-fixture",
                    1,
                    bytes,
                )),
            )
        })
        .collect();
    files.insert(
        "digests.json".into(),
        canonical_json(&serde_json::to_value(digests).unwrap()).unwrap(),
    );
    files
}
#[test]
fn frozen_reader_maps_regenerate_twice_byte_for_byte() {
    let first = corpus();
    assert_eq!(first, corpus());
    for (name, bytes) in first {
        assert_eq!(
            std::fs::read(directory().join(&name)).unwrap(),
            bytes,
            "{name}"
        );
    }
}
#[test]
#[ignore = "one-time creation; refuses to overwrite frozen corpus"]
fn create_reader_map_corpus() {
    let directory = directory();
    assert!(!directory.exists());
    std::fs::create_dir_all(&directory).unwrap();
    for (name, bytes) in corpus() {
        std::fs::write(directory.join(name), bytes).unwrap();
    }
}
