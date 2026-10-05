#![allow(clippy::unwrap_used, clippy::panic)]

mod support;

use narrata_authoring::{
    CheckedRecord, ComposeMode, PublicationStatus,
    objects::Digest,
    publication::{ObjectRole, PublicationDescription, publication_from_pack, read_publication},
    publish_records, split_source,
};
use narrata_nodes::{ContentOutline, Error, NodeRegistry, canonical_json, compile};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn inputs(records: &[CheckedRecord]) -> Vec<&[u8]> {
    records.iter().map(CheckedRecord::bytes).collect()
}
fn outlines() -> Vec<ContentOutline> {
    let text = std::fs::read_to_string(
        support::repository().join("products/gamebook-demo/content/zh-Hans.json"),
    )
    .unwrap();
    let pack = narrata_content_local::ContentPack::parse(&text).unwrap();
    vec![narrata_content_local::outline(&pack)]
}
fn digest(bytes: &[u8]) -> Digest {
    Digest::from_bytes(narrata_kernel::codec::digest_bytes(
        "narrata.nodes.publication",
        1,
        bytes,
    ))
}
fn object_map(publication: &narrata_authoring::CheckedPublication) -> BTreeMap<Digest, Vec<u8>> {
    publication
        .objects()
        .map(|(r, b)| (r.object_id, b.to_vec()))
        .collect()
}
fn read(
    bytes: &[u8],
    map: &BTreeMap<Digest, Vec<u8>>,
) -> narrata_nodes::Result<narrata_authoring::CheckedPublication> {
    read_publication(bytes, digest(bytes), |id| {
        map.get(&id)
            .cloned()
            .ok_or_else(|| Error::new("missing_object", id.to_string(), "missing"))
    })
}

#[test]
fn records_and_old_pack_publish_identical_checked_objects_and_repack_without_changes() {
    let source = support::demo().unwrap();
    let records = split_source(&source).into_result().unwrap();
    let first = publish_records(&inputs(&records), &outlines(), None, ComposeMode::Update)
        .into_result()
        .unwrap();
    assert_eq!(first.status, PublicationStatus::Switchable);
    assert!(!first.compared);
    assert_eq!(first.previous_artifact_id, None);
    let old_pack =
        std::fs::read(support::repository().join("products/gamebook-demo/story.narpack")).unwrap();
    let old = publication_from_pack(&old_pack).unwrap();
    assert_eq!(
        old.description_bytes(),
        first.publication.description_bytes()
    );
    assert_eq!(old.pack().unwrap(), old_pack);
    let reopened = read(
        first.publication.description_bytes(),
        &object_map(&first.publication),
    )
    .unwrap();
    assert_eq!(reopened.pack().unwrap(), old_pack);
    assert_eq!(
        reopened.publication_id(),
        first.publication.publication_id()
    );
    assert_eq!(
        compile(&source, &NodeRegistry::gamebook()).unwrap().lock,
        first.lock
    );
    let mut reversed = records.clone();
    reversed.reverse();
    let second = publish_records(
        &inputs(&reversed),
        &outlines(),
        Some(reopened.program()),
        ComposeMode::Locked(&first.lock),
    )
    .into_result()
    .unwrap();
    assert!(second.compared);
    assert_eq!(
        second.previous_artifact_id,
        Some(first.candidate_artifact_id)
    );
    assert_eq!(
        second.publication.description_bytes(),
        first.publication.description_bytes()
    );
    assert_eq!(
        object_map(&second.publication),
        object_map(&first.publication)
    );
}

#[test]
fn compilation_and_outline_switch_policy_are_distinct_and_diagnostics_locate_ids() {
    let records = split_source(&support::demo().unwrap())
        .into_result()
        .unwrap();
    let missing = publish_records(&inputs(&records), &[], None, ComposeMode::Update);
    assert!(missing.complete);
    assert_eq!(missing.value.unwrap().status, PublicationStatus::Compiled);
    assert!(
        missing
            .diagnostics
            .iter()
            .any(|d| d.code == "outline_checks_skipped")
    );
    let mut broken = outlines();
    for unit in broken[0].units.values_mut() {
        unit.blocks.clear();
    }
    let report = publish_records(&inputs(&records), &broken, None, ComposeMode::Update);
    assert_eq!(
        report.value.as_ref().unwrap().status,
        PublicationStatus::Compiled
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "outline_anchor_missing"
                && d.location.node.is_some()
                && d.location.record.is_some())
    );
    let duplicate = publish_records(
        &inputs(&records),
        &[broken[0].clone(), broken[0].clone()],
        None,
        ComposeMode::Update,
    );
    assert!(duplicate.value.is_none());
    assert!(
        duplicate
            .diagnostics
            .iter()
            .any(|d| d.code == "outline_provider")
    );
    let mut malformed = records
        .iter()
        .map(|r| r.bytes().to_vec())
        .collect::<Vec<_>>();
    malformed.push(b"{bad json".to_vec());
    let report = publish_records(
        &malformed.iter().map(Vec::as_slice).collect::<Vec<_>>(),
        &outlines(),
        None,
        ComposeMode::Update,
    );
    assert!(report.value.is_none());
}

#[test]
fn descriptions_objects_and_reference_closure_reject_bad_inputs() {
    let publication = publication_from_pack(
        &std::fs::read(support::repository().join("products/gamebook-demo/story.narpack")).unwrap(),
    )
    .unwrap();
    let map = object_map(&publication);
    let original: Value = serde_json::from_slice(publication.description_bytes()).unwrap();
    for mutate in [
        |v: &mut Value| {
            v["format_version"] = json!(2);
        },
        |v: &mut Value| {
            v["unknown"] = json!(true);
        },
        |v: &mut Value| {
            v["summary"] = json!("0".repeat(64));
        },
        |v: &mut Value| {
            let duplicate = v["objects"][0].clone();
            v["objects"].as_array_mut().unwrap().insert(0, duplicate);
        },
        |v: &mut Value| {
            v["objects"].as_array_mut().unwrap().reverse();
        },
        |v: &mut Value| {
            v["objects"][0]["role"] = json!("unknown");
        },
        |v: &mut Value| {
            v["objects"][0]["bytes"] = json!(1);
        },
        |v: &mut Value| {
            v["objects"][0]["encoding"] = json!("json");
        },
        |v: &mut Value| {
            v["objects"][0]["object_id"] = json!("ff");
        },
        |v: &mut Value| {
            v["objects"].as_array_mut().unwrap().remove(0);
        },
    ] {
        let mut value = original.clone();
        mutate(&mut value);
        assert!(read(&canonical_json(&value).unwrap(), &map).is_err());
    }
    let duplicate_key = String::from_utf8(publication.description_bytes().to_vec())
        .unwrap()
        .replacen("{", "{\"format_version\":1,", 1);
    assert!(read(duplicate_key.as_bytes(), &map).is_err());
    let mut broken = map.clone();
    let chunk = publication
        .description()
        .objects
        .iter()
        .find(|r| r.role == ObjectRole::Chunk)
        .unwrap()
        .object_id;
    broken.get_mut(&chunk).unwrap()[60] ^= 1;
    assert!(read(publication.description_bytes(), &broken).is_err());
    broken = map.clone();
    broken.remove(&chunk);
    assert!(read(publication.description_bytes(), &broken).is_err());
    let mut noncanonical = publication.description_bytes().to_vec();
    noncanonical.push(b' ');
    assert!(read(&noncanonical, &map).is_err());
    assert!(
        read_publication(
            publication.description_bytes(),
            Digest::from_bytes([0; 32]),
            |id| Ok(map[&id].clone())
        )
        .is_err()
    );
    let _: PublicationDescription =
        serde_json::from_slice(publication.description_bytes()).unwrap();
}

#[test]
fn old_and_new_publications_are_isolated_and_retries_are_idempotent() {
    let source = support::demo().unwrap();
    let records = split_source(&source).into_result().unwrap();
    let old = publish_records(&inputs(&records), &outlines(), None, ComposeMode::Update)
        .into_result()
        .unwrap();
    let mut changed = source.clone();
    changed
        .packages
        .get_mut("road")
        .unwrap()
        .graphs
        .get_mut("crossing")
        .unwrap()
        .nodes
        .get_mut("rocks")
        .unwrap()
        .data["body"]["unit"]["key"] = json!("revision-two");
    let new_records = split_source(&changed).into_result().unwrap();
    let publish_new = || {
        publish_records(
            &inputs(&new_records),
            &[],
            Some(old.publication.program()),
            ComposeMode::Update,
        )
        .into_result()
        .unwrap()
    };
    let new = publish_new();
    let retry = publish_new();
    assert_ne!(
        new.publication.publication_id(),
        old.publication.publication_id()
    );
    assert_eq!(
        new.publication.publication_id(),
        retry.publication.publication_id()
    );
    let mut store = object_map(&old.publication);
    for (id, bytes) in object_map(&new.publication) {
        if let Some(previous) = store.insert(id, bytes.clone()) {
            assert_eq!(previous, bytes);
        }
    }
    assert_eq!(
        read(old.publication.description_bytes(), &store)
            .unwrap()
            .program()
            .artifact_id(),
        old.candidate_artifact_id
    );
    assert_eq!(
        read(new.publication.description_bytes(), &store)
            .unwrap()
            .program()
            .artifact_id(),
        new.candidate_artifact_id
    );
    let mut mixed = new.publication.description().clone();
    mixed.relation_index = old.publication.description().relation_index;
    assert!(read(&canonical_json(&mixed).unwrap(), &store).is_err());
    // No body bytes are an input: changing or removing content cannot change reconstruction.
    let rebuilt = publication_from_pack(&old.publication.pack().unwrap()).unwrap();
    assert_eq!(rebuilt.publication_id(), old.publication.publication_id());
}

#[test]
fn schema_and_ddl_are_the_authoritative_generated_contracts() {
    for (name, schema) in narrata_authoring::schemas()
        .into_iter()
        .filter(|(name, _)| name.starts_with("publication") || name.starts_with("relation"))
    {
        let checked: Value = serde_json::from_slice(
            &std::fs::read(
                support::repository()
                    .join("packages/narrata/nodes/schemas")
                    .join(name),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(checked, serde_json::to_value(schema).unwrap(), "{name}");
    }
    let adr = std::fs::read_to_string(
        support::repository().join("docs/adr/0022-authoring-publication-and-reader-map.md"),
    )
    .unwrap()
    .replace("\r\n", "\n");
    let sql = adr
        .split_once("```sql\n")
        .unwrap()
        .1
        .split_once("\n```")
        .unwrap()
        .0;
    assert_eq!(narrata_authoring::relations::REFERENCE_DDL.trim_end(), sql);
}

#[test]
fn content_text_and_language_are_independent_of_all_publication_objects() {
    let text = std::fs::read_to_string(
        support::repository().join("products/gamebook-demo/content/zh-Hans.json"),
    )
    .unwrap();
    let original = narrata_content_local::ContentPack::parse(&text).unwrap();
    let mut translated = original.clone();
    translated.language = "en".into();
    for entry in translated.entries.values_mut() {
        match entry {
            narrata_content_local::Entry::Text(entry) => {
                entry.text = "Translated title or label".into()
            }
            narrata_content_local::Entry::Blocks(entry) => {
                for block in &mut entry.blocks {
                    if let narrata_content_local::Block::Text { text, .. } = block {
                        *text = "Completely different body text".into();
                    }
                }
            }
        }
    }
    let records = split_source(&support::demo().unwrap())
        .into_result()
        .unwrap();
    let publish = |content| {
        publish_records(
            &inputs(&records),
            &[narrata_content_local::outline(content)],
            None,
            ComposeMode::Update,
        )
        .into_result()
        .unwrap()
    };
    let first = publish(&original);
    let second = publish(&translated);
    assert_eq!(
        first.publication.description_bytes(),
        second.publication.description_bytes()
    );
    assert_eq!(
        object_map(&first.publication),
        object_map(&second.publication)
    );
}

#[test]
fn tombstone_patches_are_explicit_persistable_and_required_for_locked_publication() {
    let source = support::demo().unwrap();
    let records = split_source(&source).into_result().unwrap();
    let previous = publish_records(&inputs(&records), &[], None, ComposeMode::Update)
        .into_result()
        .unwrap();
    let mut edit = source.clone();
    edit.packages
        .get_mut("road")
        .unwrap()
        .graphs
        .get_mut("crossing")
        .unwrap()
        .nodes
        .get_mut("rocks")
        .unwrap()
        .data["choice_points"][0]["options"]
        .as_array_mut()
        .unwrap()
        .remove(1);
    let records = split_source(&edit).into_result().unwrap();
    let published = publish_records(
        &inputs(&records),
        &[],
        Some(previous.publication.program()),
        ComposeMode::Update,
    )
    .into_result()
    .unwrap();
    assert_eq!(published.tombstones.len(), 1);
    assert!(
        publish_records(
            &inputs(&records),
            &[],
            Some(previous.publication.program()),
            ComposeMode::Locked(&published.lock)
        )
        .value
        .is_none()
    );
    let persisted = publish_records(
        &inputs(&published.records),
        &[],
        Some(previous.publication.program()),
        ComposeMode::Locked(&published.lock),
    )
    .into_result()
    .unwrap();
    assert!(persisted.tombstones.is_empty());
    assert_eq!(
        persisted.publication.publication_id(),
        published.publication.publication_id()
    );
}
