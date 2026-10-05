//! Freeze the ADR 0017 messages and whole-store export in an empty output directory.
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use narrata_kernel::codec::sha256;
use narrata_storage::{KeyValue, ObjectDigest, Revision};
use narrata_storage_host::{Flush, FlushReply, LoadRequest, Loaded, StoreExport};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

fn main() {
    let output = PathBuf::from(std::env::args().nth(1).expect("empty output directory"));
    std::fs::create_dir_all(&output).unwrap();
    assert!(
        std::fs::read_dir(&output).unwrap().next().is_none(),
        "never overwrite a frozen corpus"
    );
    // These independently encoded vectors already served Rust/TS tests before the freeze.
    let vector_bytes = include_bytes!("../tests/protocol-vectors.json");
    let vectors: Value = serde_json::from_slice(vector_bytes).unwrap();
    std::fs::write(output.join("protocol-vectors.json"), vector_bytes).unwrap();
    let mut artifacts = serde_json::Map::new();
    artifacts.insert(
        "protocol-vectors.json".to_owned(),
        json!({ "sha256": hex::encode(sha256(vector_bytes)) }),
    );
    for name in ["loadRequest", "loaded", "flush", "persisted", "conflict"] {
        let bytes = hex::decode(vectors[name]["hex"].as_str().unwrap()).unwrap();
        let encoded = match name {
            "loadRequest" => LoadRequest::decode(&bytes).unwrap().encode(),
            "loaded" => Loaded::decode(&bytes).unwrap().encode(),
            "flush" => Flush::decode(&bytes).unwrap().encode(),
            _ => FlushReply::decode(&bytes).unwrap().encode(),
        };
        assert_eq!(bytes, encoded);
        let file = format!("{name}.cbor");
        std::fs::write(output.join(&file), &bytes).unwrap();
        artifacts.insert(
            file,
            json!({ "bytes": bytes.len(), "sha256": hex::encode(sha256(&bytes)) }),
        );
    }
    let exported = StoreExport {
        revision: 9,
        objects: vec![
            (ObjectDigest::from_bytes([0x11; 32]), Arc::from([1, 2, 3])),
            (ObjectDigest::from_bytes([0x22; 32]), Arc::from([])),
        ],
        keys: vec![
            (
                b"\0\0layout".to_vec(),
                KeyValue {
                    value: vec![0xa1, 0, 1],
                    revision: Revision::new(1).unwrap(),
                },
            ),
            (
                b"\0\x04abc".to_vec(),
                KeyValue {
                    value: vec![0x76],
                    revision: Revision::new(9).unwrap(),
                },
            ),
            (
                b"\0\x04abd".to_vec(),
                KeyValue {
                    value: vec![],
                    revision: Revision::new(2).unwrap(),
                },
            ),
        ],
    };
    let bytes = exported.encode();
    assert_eq!(StoreExport::decode(&bytes).unwrap(), exported);
    std::fs::write(output.join("store-export.cbor"), &bytes).unwrap();
    artifacts.insert(
        "store-export.cbor".to_owned(),
        json!({ "bytes": bytes.len(), "sha256": hex::encode(sha256(&bytes)) }),
    );
    let manifest = json!({
        "corpus_version": 1, "adr": "0017", "protocol_version": 1, "indexeddb_version": 1, "export_version": 1,
        "artifacts": artifacts,
        "export": { "revision": exported.revision,
            "objects": exported.objects.iter().map(|(digest, bytes)| json!([hex::encode(digest.as_bytes()), hex::encode(bytes)])).collect::<Vec<_>>(),
            "keys": exported.keys.iter().map(|(key, value)| json!([hex::encode(key), hex::encode(&value.value), value.revision.get()])).collect::<Vec<_>>() },
        "build_provenance": { "example": "emit_browser_storage_v1", "object_bytes": "opaque storage-contract values, not history envelopes" },
    });
    std::fs::write(
        output.join("manifest.json"),
        format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
    )
    .unwrap();
}
