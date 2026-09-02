#![allow(clippy::panic, clippy::unwrap_used)]

use narrata_core::{
    codec::DecodeError,
    program::{decode_program_artifact, load_program},
    snapshot::{SnapshotRestoreError, restore_snapshot},
};

fn frozen(name: &str) -> Vec<u8> {
    let text = match name {
        "program" => include_str!("../../../fixtures/compat/stage5-v0/program-v0.hex"),
        "snapshot" => include_str!("../../../fixtures/compat/stage5-v0/snapshot-v0.hex"),
        _ => panic!("unknown fixture"),
    };
    hex::decode(text.trim()).unwrap()
}

#[test]
fn envelope_program_and_snapshot_versions_dispatch_independently() {
    let mut envelope = frozen("program");
    envelope[8..10].copy_from_slice(&7_u16.to_be_bytes());
    assert!(matches!(
        decode_program_artifact(&envelope, &Default::default()),
        Err(DecodeError::UnsupportedVersion {
            axis: "envelope",
            version: 7
        })
    ));

    let mut program_schema = frozen("program");
    program_schema[12..14].copy_from_slice(&8_u16.to_be_bytes());
    assert!(matches!(
        decode_program_artifact(&program_schema, &Default::default()),
        Err(DecodeError::UnsupportedVersion {
            axis: "object schema",
            version: 8
        })
    ));

    let program = load_program(&frozen("program"), &Default::default()).unwrap();
    let mut snapshot_schema = frozen("snapshot");
    snapshot_schema[12..14].copy_from_slice(&9_u16.to_be_bytes());
    assert!(matches!(
        restore_snapshot(&snapshot_schema, &program, &Default::default()),
        Err(SnapshotRestoreError::Decode(
            DecodeError::UnsupportedVersion {
                axis: "object schema",
                version: 9
            }
        ))
    ));
}
