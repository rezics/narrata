//! Writes the frozen save layout v1 corpus under `fixtures/compat/store-layout-v1`: a SQLite
//! save database written by the save engine, and a manifest of every object and key in it.
//!
//! `cargo run -p narrata-store-sqlite --example emit_store_layout_v1_fixture -- <output-directory>`

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

#[path = "../tests/corpus/mod.rs"]
mod corpus;

use std::{path::PathBuf, sync::Arc};

use narrata_core::{
    CapabilityId, ContentLockId, ExecutionId, HostSnapshotDigest, InputId, ObjectId, Value,
    builtin_host_capabilities,
    codec::sha256,
    program::{encode_program_artifact, load_program},
    runtime::CheckedRuntimeInput,
};
use narrata_store::{
    BranchId, CommitTransaction, FederatedSaveVerifier, HostSnapshotRef, HostSnapshotVerifier,
    InitialRecordingMode, LeaseId, Pin, RefName, SaveStore, SessionCoordinator,
    TimelineOperationId,
};
use narrata_testkit::generator::{hello_v0, recorded_query_v0};
use serde_json::json;

const DATABASE: &str = "saves.sqlite3";

/// Accepts every host snapshot and content lock; the corpus records the Compound Save, not the
/// host's verification.
struct AcceptAll;

impl HostSnapshotVerifier for AcceptAll {
    fn verify(&self, _: &HostSnapshotRef) -> Result<(), String> {
        Ok(())
    }
}

impl FederatedSaveVerifier for AcceptAll {
    fn verify_content_lock(&self, _: ContentLockId) -> Result<(), String> {
        Ok(())
    }
}

fn main() {
    let output = PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&output).unwrap();
    let path = output.join(DATABASE);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }

    let program = load_program(
        &encode_program_artifact(&recorded_query_v0().unwrap()),
        &Default::default(),
    )
    .unwrap();
    let execution = ExecutionId::from_u128(0x1a71);
    let session = RefName::new("layout-v1-session").unwrap();
    let host = builtin_host_capabilities().unwrap();
    let mut coordinator = SessionCoordinator::create_with_capabilities(
        narrata_store_sqlite::open(&path).unwrap(),
        Arc::clone(&program),
        execution,
        session.clone(),
        BranchId::from_u128(1),
        InitialRecordingMode::Complete,
        1,
        &host,
    )
    .unwrap();
    let genesis = coordinator.timeline().cursor;
    let pending = coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap()
        .commit;
    let lease = LeaseId::from_u128(7);
    coordinator.claim_pending_effect(lease, 3, 30).unwrap();
    coordinator
        .complete_pending_effect(lease, Value::I64(42), 4)
        .unwrap();
    coordinator
        .recover_pending_effect(Default::default(), 5)
        .unwrap()
        .unwrap();
    let player = RefName::new("player").unwrap();
    coordinator
        .save(
            player.clone(),
            RefName::new("slot-1").unwrap(),
            None,
            TimelineOperationId::from_u128(10),
            6,
        )
        .unwrap();
    coordinator
        .bookmark(
            player.clone(),
            RefName::new("mark").unwrap(),
            TimelineOperationId::from_u128(11),
            7,
        )
        .unwrap();
    coordinator
        .fork(
            BranchId::from_u128(2),
            TimelineOperationId::from_u128(12),
            8,
        )
        .unwrap();
    coordinator
        .save_compound(
            player,
            RefName::new("compound").unwrap(),
            HostSnapshotRef::new(
                CapabilityId::new("host.snapshot").unwrap(),
                "snapshot-1",
                1,
                HostSnapshotDigest::from_bytes([1; 32]),
            )
            .unwrap(),
            ContentLockId::from_bytes([3; 32]),
            None,
            &AcceptAll,
            9,
        )
        .unwrap();
    coordinator
        .store_mut()
        .commit(CommitTransaction {
            pins: vec![
                Pin {
                    owner: "fixture-pin".to_owned(),
                    object: ObjectId::from_bytes(*pending.as_bytes()),
                    expires_at: Some(1_000),
                },
                Pin {
                    owner: "fixture-pin".to_owned(),
                    object: ObjectId::from_bytes(*genesis.as_bytes()),
                    expires_at: None,
                },
            ],
            observed_at: 10,
            ..CommitTransaction::default()
        })
        .unwrap();
    let selected = coordinator.timeline();

    // A second execution, recorded completely and sealed into a timeline archive.
    let hello = load_program(&encode_program_artifact(&hello_v0()), &Default::default()).unwrap();
    let mut archived = SessionCoordinator::create(
        coordinator.into_store(),
        hello,
        ExecutionId::from_u128(0x1a72),
        RefName::new("archived-session").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Complete,
        11,
    )
    .unwrap();
    archived
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            12,
        )
        .unwrap();
    archived
        .seal_complete_recording(RefName::new("sealed").unwrap(), 13)
        .unwrap();
    let store = archived.into_store();
    let image = corpus::layout_image(&store);
    let integrity = store.integrity_scan().unwrap();
    assert!(integrity.is_empty(), "{integrity:?}");
    drop(store);

    let bytes = std::fs::read(&path).unwrap();
    let manifest = json!({
        "corpus_version": 1,
        "format": "narrata save layout v1 (SQLite backend)",
        "artifacts": { DATABASE: { "sha256": hex::encode(sha256(&bytes)) } },
        "program_artifact_id": program.artifact_id().to_string(),
        "session": {
            "execution": execution.to_string(),
            "name": session.as_str(),
            "selected_branch": selected.selected_branch.to_string(),
            "cursor": selected.cursor.to_string(),
            "genesis": genesis.to_string(),
            "pending_effect_commit": pending.to_string(),
        },
        "image": image,
        "build_provenance": {
            "writer": "narrata-store-sqlite/0.1.0-alpha.1 save layout v1",
            "source": "generator:recorded-query-v0, generator:hello-v0",
            "command": "cargo run -p narrata-store-sqlite --example emit_store_layout_v1_fixture -- fixtures/compat/store-layout-v1",
        },
    });
    std::fs::write(
        output.join("manifest.json"),
        format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
    )
    .unwrap();
}
