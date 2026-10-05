use std::{collections::BTreeSet, error::Error, fs, path::PathBuf};

use narrata_graph::{
    ClusterId, Id, MAX_NODES, decode_labels, encode_labels,
    wire::{LABEL_KIND, SCHEMA_VERSION},
};
use narrata_kernel::{
    codec::{CborWriter, encode_envelope, object_id, sha256},
    content::ContentRef,
};
use proptest::prelude::*;
use serde::Deserialize;

#[path = "support/labels.rs"]
mod support;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format_version: u16,
    files: Vec<File>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    cluster: ClusterId,
    object_id: String,
    file: String,
    bytes: usize,
    sha256: String,
}

#[test]
fn frozen_labels_decode_and_regenerate_byte_for_byte() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../fixtures/compat/graph-v1-labels");
    let manifest: Manifest = serde_json::from_slice(&fs::read(root.join("manifest.json"))?)?;
    assert_eq!(manifest.format_version, 1);
    let tables = support::tables();
    assert_eq!(manifest.files.len(), tables.len());
    let mut files = BTreeSet::from(["manifest.json".to_owned()]);
    for (file, table) in manifest.files.iter().zip(tables) {
        let generated = encode_labels(&table)?;
        assert_eq!(file.cluster, table.cluster);
        assert_eq!(file.object_id, hex::encode(generated.id));
        assert_eq!(file.file, generated.filename());
        assert!(files.insert(file.file.clone()));
        let bytes = fs::read(root.join(&file.file))?;
        assert_eq!(bytes.len(), file.bytes);
        assert_eq!(hex::encode(sha256(&bytes)), file.sha256);
        assert_eq!(bytes, generated.bytes);
        assert_eq!(decode_labels(&bytes, &generated.id, file.cluster)?, table);
    }
    let actual_files: BTreeSet<_> = fs::read_dir(root)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()?;
    assert_eq!(files, actual_files);
    Ok(())
}

#[test]
fn title_changes_change_only_the_label_identity() -> Result<(), Box<dyn Error>> {
    let mut table = support::tables().remove(0);
    let before = encode_labels(&table)?;
    table.titles.insert(
        Id::from_u128(2),
        ContentRef::new("local", "different:title")?,
    );
    let after = encode_labels(&table)?;
    assert_ne!(before.id, after.id);
    assert!(decode_labels(&after.bytes, &before.id, table.cluster).is_err());
    assert!(decode_labels(&after.bytes, &after.id, ClusterId::from_u128(9)).is_err());
    Ok(())
}

fn raw(rows: &[(Id, &str, &str)]) -> Vec<u8> {
    let mut writer = CborWriter::new();
    writer.map(2);
    writer.unsigned(0);
    writer.bytes(&ClusterId::from_u128(1).0);
    writer.unsigned(1);
    writer.array(rows.len() as u64);
    for (id, provider, key) in rows {
        writer.array(2);
        writer.bytes(&id.0);
        writer.map(2);
        writer.unsigned(0);
        writer.text(provider);
        writer.unsigned(1);
        writer.text(key);
    }
    writer.into_bytes()
}

#[test]
fn recomputed_checksums_do_not_admit_bad_rows_fields_or_versions() {
    let id = Id::from_u128(2);
    let next = Id::from_u128(3);
    let valid = raw(&[(id, "local", "title")]);
    let mut nonminimal = valid.clone();
    nonminimal.splice(1..2, [0x18, 0]);
    let mut unknown = valid.clone();
    unknown[1] = 2;
    let mut duplicate = valid.clone();
    duplicate[0] = 0xa3;
    duplicate.extend_from_slice(&[1, 0x80]);
    let mut trailing = valid.clone();
    trailing.push(0);
    let mut indefinite = valid.clone();
    indefinite[0] = 0xbf;
    indefinite.push(0xff);
    let mut too_many = CborWriter::new();
    too_many.map(2);
    too_many.unsigned(0);
    too_many.bytes(&ClusterId::from_u128(1).0);
    too_many.unsigned(1);
    too_many.array(MAX_NODES as u64 + 1);
    for bytes in [
        nonminimal,
        unknown,
        duplicate,
        trailing,
        indefinite,
        too_many.into_bytes(),
        raw(&[(id, "local", "title"), (id, "local", "title")]),
        raw(&[(next, "local", "title"), (id, "local", "title")]),
        raw(&[(id, "INVALID", "title")]),
        raw(&[(id, "local", "")]),
        raw(&[(id, "local", "bad\0key")]),
        raw(&[(id, "local", &"x".repeat(257))]),
    ] {
        let id = object_id(LABEL_KIND, SCHEMA_VERSION, &bytes);
        assert!(
            decode_labels(
                &encode_envelope(LABEL_KIND, SCHEMA_VERSION, &bytes),
                &id,
                ClusterId::from_u128(1)
            )
            .is_err()
        );
    }
    for (kind, schema) in [(0x0123, 1), (LABEL_KIND, 0), (LABEL_KIND, 2)] {
        let id = object_id(kind, schema, &valid);
        assert!(
            decode_labels(
                &encode_envelope(kind, schema, &valid),
                &id,
                ClusterId::from_u128(1)
            )
            .is_err()
        );
    }
}

proptest! {
    #[test]
    fn arbitrary_label_input_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
        let _ = decode_labels(&bytes, &[0; 32], ClusterId::from_u128(1));
    }

    #[test]
    fn every_single_byte_tamper_is_rejected(offset in any::<usize>(), mask in 1_u8..=255) {
        let table = support::tables().remove(0);
        let object = encode_labels(&table).map_err(|error| TestCaseError::fail(error.to_string()))?;
        let mut bytes = object.bytes;
        let offset = offset % bytes.len();
        bytes[offset] ^= mask;
        prop_assert!(decode_labels(&bytes, &object.id, table.cluster).is_err());
    }
}
