#![allow(clippy::panic, clippy::unwrap_used)]

use narrata_kernel::codec::{CborWriter, DecodeError, sha256};
use narrata_storage_host::{Flush, FlushReply, LoadRequest, Loaded, StoreExport};
use serde_json::Value;

fn artifact(file: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../fixtures/compat/browser-storage-v1");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(path.join("manifest.json")).unwrap()).unwrap();
    let bytes = std::fs::read(path.join(file)).unwrap();
    assert_eq!(
        hex::encode(sha256(&bytes)),
        manifest["artifacts"][file]["sha256"]
    );
    bytes
}

#[test]
fn frozen_protocol_messages_decode_and_reencode_exactly() {
    let vectors: Value = serde_json::from_slice(&artifact("protocol-vectors.json")).unwrap();
    for name in ["loadRequest", "loaded", "flush", "persisted", "conflict"] {
        let bytes = artifact(&format!("{name}.cbor"));
        assert_eq!(hex::encode(&bytes), vectors[name]["hex"]);
        let encoded = match name {
            "loadRequest" => LoadRequest::decode(&bytes).unwrap().encode(),
            "loaded" => Loaded::decode(&bytes).unwrap().encode(),
            "flush" => Flush::decode(&bytes).unwrap().encode(),
            _ => FlushReply::decode(&bytes).unwrap().encode(),
        };
        assert_eq!(encoded, bytes);
        let mut trailing = bytes;
        trailing.push(0);
        assert!(match name {
            "loadRequest" => LoadRequest::decode(&trailing).is_err(),
            "loaded" => Loaded::decode(&trailing).is_err(),
            "flush" => Flush::decode(&trailing).is_err(),
            _ => FlushReply::decode(&trailing).is_err(),
        });
    }
}

#[test]
fn frozen_browser_export_is_readable_by_a_native_host() {
    let bytes = artifact("store-export.cbor");
    let exported = StoreExport::decode(&bytes).unwrap();
    assert_eq!(exported.revision, 9);
    assert_eq!(exported.objects.len(), 2);
    assert_eq!(exported.objects[0].1.as_ref(), [1, 2, 3]);
    assert!(exported.objects[1].1.is_empty());
    assert_eq!(
        exported
            .keys
            .iter()
            .map(|(_, value)| value.revision.get())
            .collect::<Vec<_>>(),
        [1, 9, 2]
    );
    assert_eq!(exported.encode(), bytes);
}

#[test]
fn browser_export_decoding_refuses_bad_versions_order_revisions_and_encodings() {
    let bytes = artifact("store-export.cbor");
    let mut exported = StoreExport::decode(&bytes).unwrap();
    exported.revision = 8;
    assert!(StoreExport::decode(&exported.encode()).is_err());
    exported.revision = 1_u64 << 53;
    assert!(StoreExport::decode(&exported.encode()).is_err());
    exported.revision = 9;
    exported.keys.reverse();
    assert!(StoreExport::decode(&exported.encode()).is_err());
    exported.keys.reverse();
    exported.objects.push(exported.objects[0].clone());
    assert!(StoreExport::decode(&exported.encode()).is_err());
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(StoreExport::decode(&trailing).is_err());
    assert!(StoreExport::decode(&bytes[..bytes.len() - 1]).is_err());
    let mut writer = CborWriter::new();
    writer.array(5);
    writer.text("narrata-store-export");
    writer.unsigned(2);
    writer.unsigned(0);
    writer.array(0);
    writer.array(0);
    assert!(StoreExport::decode(&writer.into_bytes()).is_err());
    let mut noncanonical = bytes;
    assert_eq!(noncanonical[22], 1);
    noncanonical.splice(22..23, [0x18, 1]); // The version follows the 20-byte export label.
    assert!(matches!(
        StoreExport::decode(&noncanonical),
        Err(DecodeError::NonCanonical(_))
    ));
}
