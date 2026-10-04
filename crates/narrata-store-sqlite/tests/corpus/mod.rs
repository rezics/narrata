//! Descriptions of a save store that frozen corpora record and their tests compare. Shared by
//! the layout v1 fixture generator, which includes this file by path.

#![allow(dead_code, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use narrata_core::{ExecutionId, InputId, ObjectId, codec::sha256};
use narrata_storage::{ObjectDigest, StorageBackend};
use narrata_store::{
    CatalogRefKey, LedgerStatus, Pin, RefKey, RefScope, SaveStore, Store, TimelineCoverage, layout,
    scan_all,
};
use serde_json::{Value as Json, json};

const PAGE: u32 = 64;

/// Every stored object and every key of the save layout, as stored.
pub fn layout_image<B: StorageBackend>(store: &Store<B>) -> Json {
    let backend = store.backend();
    let mut digests = Vec::<ObjectDigest>::new();
    let mut after = None;
    loop {
        let page = backend.scan_objects(after.as_ref(), PAGE).unwrap();
        after = page.resume_after().copied();
        digests.extend(page.digests);
        if after.is_none() {
            break;
        }
    }
    let ids = digests
        .iter()
        .map(|digest| ObjectId::from_bytes(*digest.as_bytes()))
        .collect::<Vec<_>>();
    let objects = store
        .get_objects(&ids)
        .unwrap()
        .into_iter()
        .map(|object| {
            let object = object.unwrap();
            json!({
                "id": object.id().to_string(),
                "kind": object.kind().code(),
                "schema": object.schema(),
                "sha256": hex::encode(sha256(object.bytes())),
            })
        })
        .collect::<Vec<_>>();
    let mut keys = Vec::new();
    for (space, name) in layout::SPACES {
        let mut after: Option<Vec<u8>> = None;
        loop {
            let page = backend
                .scan_keys(space, b"", after.as_deref(), PAGE)
                .unwrap();
            after = page.resume_after().map(<[u8]>::to_vec);
            keys.extend(page.entries.into_iter().map(|entry| {
                json!({
                    "space": name,
                    "key": hex::encode(&entry.key),
                    "value": hex::encode(&entry.value),
                    "revision": entry.revision.get(),
                })
            }));
            if after.is_none() {
                break;
            }
        }
    }
    json!({ "objects": objects, "keys": keys })
}

/// The roots and ledger of one execution as the schema v2 corpus records them, without the
/// revisions, which migration does not keep. Objects are compared separately.
pub fn contents(store: &impl SaveStore, execution: ExecutionId, inputs: &[InputId]) -> Json {
    let refs = scan_all(
        |after| store.scan_refs(&RefScope::All, after, PAGE),
        |(key, _): &(RefKey, _)| key.clone(),
    )
    .unwrap()
    .iter()
    .map(|(key, value)| {
        json!({
            "namespace": key.namespace().as_str(),
            "owner": key.owner().as_str(),
            "name": key.name().as_str(),
            "commit": value.commit.to_string(),
        })
    })
    .collect::<Vec<_>>();
    let catalogs = scan_all(
        |after| store.scan_catalog_heads(after, PAGE),
        |(key, _): &(CatalogRefKey, _)| key.clone(),
    )
    .unwrap()
    .iter()
    .map(|(key, value)| {
        json!({
            "timeline": key.timeline().to_string(),
            "event": value.event.to_string(),
            "coverage": coverage(value.coverage),
        })
    })
    .collect::<Vec<_>>();
    let inputs = inputs
        .iter()
        .filter_map(|input| store.read_input(execution, *input).unwrap())
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
    let pins = scan_all(
        |after| store.scan_pins(after, PAGE),
        |pin: &Pin| (pin.owner.clone(), pin.object),
    )
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
    let effects = scan_all(
        |after| store.scan_effects(execution, after, PAGE),
        |entry: &narrata_store::EffectLedgerEntry| entry.effect,
    )
    .unwrap()
    .iter()
    .map(|entry| {
        let status = match &entry.status {
            LedgerStatus::Completed { response, fence } => json!({
                "state": "completed",
                "response": response.to_string(),
                "fence": fence.get(),
            }),
            other => json!({ "state": format!("{other:?}") }),
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

/// The corpus directory `name` under `fixtures/compat`.
pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/compat")
        .join(name)
}
