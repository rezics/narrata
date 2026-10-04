//! Writes the frozen schema v2 save database under `fixtures/compat/store-sqlite-v2`.
//!
//! Run once with the schema v2 adapter, before the engine rewrite replaces it:
//! `cargo run -p narrata-store-sqlite --example emit_store_v2_fixture -- <output-directory>`.

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::{path::PathBuf, sync::Arc};

use narrata_core::{
    ExecutionId, InputId, ObjectId, Value, builtin_host_capabilities,
    codec::sha256,
    program::{encode_program_artifact, load_program},
    runtime::CheckedRuntimeInput,
};
use narrata_store::{
    BranchId, CommitTransaction, InitialRecordingMode, LeaseId, LedgerStatus, Pin, RefName,
    SaveStore, SessionCoordinator, TimelineCoverage, TimelineOperationId,
};
use narrata_store_sqlite::SqliteStore;
use narrata_testkit::generator::recorded_query_v0;
use serde_json::{Value as Json, json};

const DATABASE: &str = "save-v2.sqlite3";

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
    let execution = ExecutionId::from_u128(0x5632);
    let session = RefName::new("v2-session").unwrap();
    let branch = BranchId::from_u128(1);
    let host = builtin_host_capabilities().unwrap();
    let mut coordinator = SessionCoordinator::create_with_capabilities(
        SqliteStore::open(&path).unwrap(),
        Arc::clone(&program),
        execution,
        session.clone(),
        branch,
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
    let resumed = coordinator
        .recover_pending_effect(Default::default(), 5)
        .unwrap()
        .unwrap()
        .commit;
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
            player,
            RefName::new("mark").unwrap(),
            TimelineOperationId::from_u128(11),
            7,
        )
        .unwrap();
    coordinator
        .fork(BranchId::from_u128(2), TimelineOperationId::from_u128(12), 8)
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
            observed_at: 9,
            ..CommitTransaction::default()
        })
        .unwrap();
    let selected = coordinator.timeline();
    assert_eq!(selected.cursor, resumed);
    let store = coordinator.into_store();
    let contents = contents(&store, execution);
    // A single self-contained file: no WAL beside it, no free pages.
    store
        .connection()
        .execute_batch("PRAGMA journal_mode=DELETE; VACUUM;")
        .unwrap();
    drop(store);

    let bytes = std::fs::read(&path).unwrap();
    let manifest = json!({
        "corpus_version": 1,
        "format": "narrata-store-sqlite schema v2",
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
        "contents": contents,
        "build_provenance": {
            "writer": "narrata-store-sqlite/0.1.0-alpha.1 schema v2",
            "source": "generator:recorded-query-v0",
            "command": "cargo run -p narrata-store-sqlite --example emit_store_v2_fixture -- fixtures/compat/store-sqlite-v2",
        },
    });
    std::fs::write(
        output.join("manifest.json"),
        format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
    )
    .unwrap();
}

fn contents(store: &SqliteStore, execution: ExecutionId) -> Json {
    let inserted_at = |id: ObjectId| -> i64 {
        store
            .connection()
            .query_row(
                "SELECT inserted_at FROM objects WHERE id = ?1",
                [id.as_bytes().as_slice()],
                |row| row.get(0),
            )
            .unwrap()
    };
    let objects = store
        .list_objects()
        .unwrap()
        .iter()
        .map(|object| {
            json!({
                "id": object.id().to_string(),
                "kind": object.kind().code(),
                "schema": object.schema(),
                "sha256": hex::encode(sha256(object.bytes())),
                "inserted_at": inserted_at(object.id()),
            })
        })
        .collect::<Vec<_>>();
    let refs = store
        .list_refs()
        .unwrap()
        .iter()
        .map(|(key, value)| {
            json!({
                "namespace": key.namespace().as_str(),
                "owner": key.owner().as_str(),
                "name": key.name().as_str(),
                "commit": value.commit.to_string(),
                "revision": value.revision.get(),
            })
        })
        .collect::<Vec<_>>();
    let catalogs = store
        .list_catalog_heads()
        .unwrap()
        .iter()
        .map(|(key, value)| {
            json!({
                "timeline": key.timeline().to_string(),
                "event": value.event.to_string(),
                "coverage": coverage(value.coverage),
                "revision": value.revision.get(),
            })
        })
        .collect::<Vec<_>>();
    let inputs = [InputId::from_u128(1)]
        .into_iter()
        .filter_map(|input| store.read_input(execution, input).unwrap())
        .map(|record| {
            json!({
                "execution": record.execution.to_string(),
                "input": record.input.to_string(),
                "parent": record.parent.to_string(),
                "payload": record.payload.to_string(),
                "commit": record.commit.to_string(),
            })
        })
        .collect::<Vec<_>>();
    let pins = store
        .list_pins()
        .unwrap()
        .iter()
        .map(|pin| {
            json!({
                "owner": pin.owner,
                "object": pin.object.to_string(),
                "expires_at": pin.expires_at,
            })
        })
        .collect::<Vec<_>>();
    let effects = store
        .list_effects(execution)
        .unwrap()
        .iter()
        .map(|entry| {
            let status = match &entry.status {
                LedgerStatus::Completed { response, fence } => json!({
                    "state": "completed",
                    "response": response.to_string(),
                    "fence": fence.get(),
                }),
                other => panic!("fixture records a completed Effect, found {other:?}"),
            };
            json!({
                "execution": entry.execution.to_string(),
                "effect": entry.effect.to_string(),
                "request_digest": entry.request_digest.to_string(),
                "capability": entry.capability.as_str(),
                "capability_version": entry.capability_version.get(),
                "origin_commit": entry.origin_commit.to_string(),
                "delivery": format!("{:?}", entry.delivery),
                "rewind": format!("{:?}", entry.rewind),
                "status": status,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "objects": objects,
        "refs": refs,
        "catalogs": catalogs,
        "archives": [],
        "compound_saves": [],
        "inputs": inputs,
        "pins": pins,
        "ledger_fences": [{
            "execution": execution.to_string(),
            "fence": store.current_ledger_fence(execution).unwrap().get(),
        }],
        "effects": effects,
    })
}

fn coverage(coverage: TimelineCoverage) -> Json {
    match coverage {
        TimelineCoverage::FromBaseline { baseline } => {
            json!({ "from_baseline": baseline.to_string() })
        }
        TimelineCoverage::Imported { baseline, source } => {
            json!({ "imported": { "baseline": baseline.to_string(), "source": source.to_string() } })
        }
    }
}
