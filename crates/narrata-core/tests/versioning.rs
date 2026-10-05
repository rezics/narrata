#![allow(clippy::panic, clippy::unwrap_used)]

use narrata_core::{
    codec::DecodeError,
    program::{decode_program_artifact, load_program},
    snapshot::{SnapshotRestoreError, restore_snapshot},
    version::{PROGRAM_FORMAT_V0, PROGRAM_FORMAT_V1, SNAPSHOT_SCHEMA_V0, SNAPSHOT_SCHEMA_V1},
};

fn frozen(name: &str) -> Vec<u8> {
    let text = match name {
        "program" => include_str!("../../../fixtures/compat/stage5-v0/program-v0.hex"),
        "snapshot" => include_str!("../../../fixtures/compat/stage5-v0/snapshot-v0.hex"),
        "program-v1" => include_str!("../../../fixtures/compat/stage6-v0/program-v1.hex"),
        "snapshot-v1" => include_str!("../../../fixtures/compat/stage6-v0/snapshot-v1.hex"),
        _ => panic!("unknown fixture"),
    };
    hex::decode(text.trim()).unwrap()
}

/// Program format 0 pairs with Snapshot schema 0 and format 1 with schema 1 (ADR 0018); both
/// frozen corpora decode, and neither schema restores against the other format.
#[test]
fn program_formats_and_snapshot_schemas_pair_up() {
    let legacy = load_program(&frozen("program"), &Default::default()).unwrap();
    let text_free = load_program(&frozen("program-v1"), &Default::default()).unwrap();
    assert_eq!(legacy.format_version(), PROGRAM_FORMAT_V0);
    assert_eq!(text_free.format_version(), PROGRAM_FORMAT_V1);
    let legacy_state = restore_snapshot(&frozen("snapshot"), &legacy, &Default::default()).unwrap();
    let state = restore_snapshot(&frozen("snapshot-v1"), &text_free, &Default::default()).unwrap();
    assert_eq!(legacy_state.snapshot_schema, SNAPSHOT_SCHEMA_V0);
    assert_eq!(state.snapshot_schema, SNAPSHOT_SCHEMA_V1);
    assert!(matches!(
        restore_snapshot(&frozen("snapshot-v1"), &legacy, &Default::default()),
        Err(SnapshotRestoreError::Incompatible(_))
    ));

    let mut relabelled = frozen("program-v1");
    relabelled[12..14].copy_from_slice(&0_u16.to_be_bytes());
    assert!(decode_program_artifact(&relabelled, &Default::default()).is_err());
    let mut relabelled = frozen("snapshot");
    relabelled[12..14].copy_from_slice(&1_u16.to_be_bytes());
    assert!(restore_snapshot(&relabelled, &legacy, &Default::default()).is_err());
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
